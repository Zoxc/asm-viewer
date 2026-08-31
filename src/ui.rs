use std::{path::PathBuf, sync::Arc};

use freya::prelude::*;
use rfd::AsyncFileDialog;

use analysis::{open_files, Assembly, Object, SpanKind, Symbol, SymbolData};

use crate::fonts::{fonts, Font};
use crate::project::{Project, Selection};

/// Height of every row in the object, symbol and instruction lists. This must stay
/// equal to the `item_size` given to each `VirtualScrollView`.
const ROW_HEIGHT: f32 = 26.0;

// Palette, carried over from the original floem styling.
const HEADER_BG: Color = Color::from_rgb(245, 245, 245); // WHITE_SMOKE
const HAIRLINE: Color = Color::from_rgb(211, 211, 211); // LIGHT_GRAY
const SELECTED_BG: Color = Color::from_rgb(211, 211, 211);
const OBJECT_HOVER_BG: Color = Color::from_rgb(144, 238, 144); // LIGHT_GREEN
const SYMBOL_PANE_BG: Color = Color::from_rgb(243, 243, 228);
const SYMBOL_HOVER_BG: Color = Color::from_rgb(226, 226, 205);
const ASM_PANE_BG: Color = Color::from_rgb(248, 248, 248);
const ASM_ROW_HOVER_BG: Color = Color::from_argb(160, 228, 237, 216);
const ADDRESS_FG: Color = Color::from_rgb(118, 141, 169);
const MNEMONIC_FG: Color = Color::from_rgb(116, 94, 147);
const REGISTER_FG: Color = Color::from_rgb(87, 103, 65);
const NUMBER_FG: Color = Color::from_rgb(80, 107, 135);
const OTHER_FG: Color = Color::from_rgb(102, 102, 102);
const RELOC_FG: Color = Color::from_rgb(50, 50, 50);
const RELOC_HOVER_FG: Color = Color::from_rgb(105, 89, 132);
/// The wash over the half of a panel a dragged tab would land in.
const DROP_PREVIEW_BG: Color = Color::from_argb(60, 105, 89, 132);

/// Applying one of the two fonts. freya takes font families one at a time, pushing
/// each onto the element's own list and appending the parent's behind it, so the
/// chain is set by calling `font_family` in order of preference.
trait FontExt: TextStyleExt + Sized {
    fn font(mut self, font: &'static Font) -> Self {
        for family in &font.families {
            self = self.font_family(family.clone());
        }
        self.font_size(font.size)
    }

    /// The desktop's interface font, set on the root and inherited by everything.
    fn interface_font(self) -> Self {
        self.font(&fonts().ui)
    }

    /// The desktop's fixed-width font, for the assembly rows.
    fn assembly_font(self) -> Self {
        self.font(&fonts().mono)
    }
}

impl<T: TextStyleExt + Sized> FontExt for T {}

/// The loaded objects, shared through context.
#[derive(Clone, Copy)]
struct Objects(State<Vec<Arc<Object>>>);

/// The current selection, shared through context.
#[derive(Clone, Copy)]
struct Sel(State<Selection>);

/// The flattened symbol list, shared through context so the Symbols tab does not
/// have to rebuild it and the root does not have to re-render to hand it over.
#[derive(Clone, Copy)]
struct Symbols(Memo<SymbolList>);

/// Every object's text symbols flattened into one list, rebuilt only when the object
/// list changes. Compared by pointer so passing it around stays O(1).
#[derive(Clone)]
struct SymbolList(Arc<Vec<Symbol>>);

impl PartialEq for SymbolList {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// A disassembled symbol, compared by pointer.
#[derive(Clone)]
struct AsmData {
    assembly: Arc<Assembly>,
    object: Arc<Object>,
}

impl PartialEq for AsmData {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.assembly, &other.assembly) && Arc::ptr_eq(&self.object, &other.object)
    }
}

fn bottom_hairline() -> Border {
    Border::new().fill(HAIRLINE).width(BorderWidth {
        top: 0.0,
        right: 0.0,
        bottom: 0.5,
        left: 0.0,
    })
}

fn right_hairline() -> Border {
    Border::new().fill(HAIRLINE).width(BorderWidth {
        top: 0.0,
        right: 0.5,
        bottom: 0.0,
        left: 0.0,
    })
}

/// The body of a tab that has nothing to show.
fn placeholder(text: &'static str) -> Element {
    rect()
        .expanded()
        .padding(5.0)
        .background(Color::WHITE)
        .child(label().text(text))
        .into()
}

fn info_line(text: String) -> impl IntoElement {
    rect().padding(5.0).child(label().text(text))
}

fn kind_color(kind: SpanKind) -> Color {
    match kind {
        SpanKind::Mnemonic | SpanKind::Prefix => MNEMONIC_FG,
        SpanKind::Register => REGISTER_FG,
        SpanKind::Number => NUMBER_FG,
        SpanKind::Address => ADDRESS_FG,
        SpanKind::Other => OTHER_FG,
    }
}

// ---------------------------------------------------------------------------
// Rows
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct ObjectRow {
    object: Arc<Object>,
    selected: bool,
    key: DiffKey,
}

impl PartialEq for ObjectRow {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.object, &other.object) && self.selected == other.selected
    }
}

impl KeyExt for ObjectRow {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for ObjectRow {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let selection = use_consume::<Sel>().0;
        let object = self.object.clone();

        let background = if self.selected {
            SELECTED_BG
        } else if hovering() {
            OBJECT_HOVER_BG
        } else {
            Color::TRANSPARENT
        };

        rect()
            .width(Size::fill())
            .height(Size::px(ROW_HEIGHT))
            .padding(5.0)
            .background(background)
            .overflow(Overflow::Clip)
            .on_pointer_over(move |_| hovering.set_if_modified(true))
            .on_pointer_out(move |_| hovering.set_if_modified(false))
            .on_press(move |_| {
                let mut selection = selection;
                selection.set(Selection::Object(object.clone()));
            })
            .child(label().text(self.object.name.clone()).max_lines(1))
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

#[derive(Clone)]
struct SymbolRow {
    symbols: SymbolList,
    index: usize,
    selected: bool,
    key: DiffKey,
}

impl PartialEq for SymbolRow {
    fn eq(&self, other: &Self) -> bool {
        self.symbols == other.symbols
            && self.index == other.index
            && self.selected == other.selected
    }
}

impl KeyExt for SymbolRow {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for SymbolRow {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let selection = use_consume::<Sel>().0;
        let symbol = self.symbols.0[self.index].clone();
        let text = symbol
            .data
            .demangled
            .as_ref()
            .unwrap_or(&symbol.data.name)
            .clone();

        let background = if self.selected {
            SELECTED_BG
        } else if hovering() {
            SYMBOL_HOVER_BG
        } else {
            Color::TRANSPARENT
        };

        TooltipContainer::new(Tooltip::new(text.clone())).child(
            rect()
                .width(Size::fill())
                .height(Size::px(ROW_HEIGHT))
                .padding(5.0)
                .background(background)
                .overflow(Overflow::Clip)
                .on_pointer_over(move |_| hovering.set_if_modified(true))
                .on_pointer_out(move |_| hovering.set_if_modified(false))
                .on_press(move |_| {
                    let mut selection = selection;
                    selection.set(Selection::Symbol(symbol.clone()));
                })
                .child(label().text(text).max_lines(1)),
        )
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

/// The clickable name of a relocation target, rendered in place of the meaningless
/// numeric operand.
#[derive(Clone)]
struct RelocationLabel {
    object: Arc<Object>,
    target: Arc<SymbolData>,
}

impl PartialEq for RelocationLabel {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.object, &other.object) && Arc::ptr_eq(&self.target, &other.target)
    }
}

impl Component for RelocationLabel {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let selection = use_consume::<Sel>().0;
        let symbol = Symbol {
            object: self.object.clone(),
            data: self.target.clone(),
        };
        let text = self
            .target
            .demangled
            .as_ref()
            .unwrap_or(&self.target.name)
            .clone();

        CursorArea::new().child(
            rect()
                .maybe(hovering(), |rect| {
                    rect.background(Color::from_af32rgb(0.6, 255, 255, 255))
                        .corner_radius(6.0)
                        .border(Border::new().fill(RELOC_HOVER_FG).width(BorderWidth {
                            top: 0.0,
                            right: 0.0,
                            bottom: 2.0,
                            left: 0.0,
                        }))
                })
                .on_pointer_over(move |_| hovering.set_if_modified(true))
                .on_pointer_out(move |_| hovering.set_if_modified(false))
                .on_press(move |_| {
                    let mut selection = selection;
                    selection.set(Selection::Symbol(symbol.clone()));
                })
                .child(label().text(text).max_lines(1).color(if hovering() {
                    RELOC_HOVER_FG
                } else {
                    RELOC_FG
                })),
        )
    }
}

#[derive(Clone)]
struct InstructionRow {
    data: AsmData,
    index: usize,
    key: DiffKey,
}

impl PartialEq for InstructionRow {
    fn eq(&self, other: &Self) -> bool {
        self.data == other.data && self.index == other.index
    }
}

impl KeyExt for InstructionRow {
    fn write_key(&mut self) -> &mut DiffKey {
        &mut self.key
    }
}

impl Component for InstructionRow {
    fn render(&self) -> impl IntoElement {
        let mut hovering = use_state(|| false);
        let instruction = &self.data.assembly.instructions[self.index];

        // When an operand is dropped for a relocation the formatter's padding to the
        // operand column is left as a trailing run of spaces, and Skia trims trailing
        // whitespace when it measures a paragraph — which butts the target name right
        // up against the mnemonic. Make that padding non-breaking to keep the column.
        let pad_tail = instruction.relocation.is_some();
        let last = instruction.format.len().saturating_sub(1);

        let spans = instruction
            .format
            .iter()
            .enumerate()
            .map(move |(i, (text, kind))| {
                let text = if pad_tail && i == last {
                    let kept = text.trim_end_matches(' ');
                    format!("{kept}{}", "\u{a0}".repeat(text.len() - kept.len()))
                } else {
                    text.clone()
                };

                Span::new(text)
                    .color(kind_color(*kind))
                    .assembly_font()
                    .font_weight(if *kind == SpanKind::Mnemonic {
                        FontWeight::BOLD
                    } else {
                        FontWeight::NORMAL
                    })
            });

        let relocation = instruction
            .relocation
            .as_ref()
            .map(|target| RelocationLabel {
                object: self.data.object.clone(),
                target: target.clone(),
            });

        rect()
            .horizontal()
            .cross_align(Alignment::Center)
            .width(Size::fill())
            .height(Size::px(ROW_HEIGHT))
            .padding(3.0)
            .assembly_font()
            .background(if hovering() {
                ASM_ROW_HOVER_BG
            } else {
                Color::TRANSPARENT
            })
            .on_pointer_over(move |_| hovering.set_if_modified(true))
            .on_pointer_out(move |_| hovering.set_if_modified(false))
            .child(
                label()
                    .text(format!("{:016X} ", instruction.address))
                    .min_width(Size::px(200.0))
                    .color(ADDRESS_FG)
                    .max_lines(1),
            )
            .child(paragraph().max_lines(1).spans_iter(spans))
            .maybe_child(relocation)
    }

    fn render_key(&self) -> DiffKey {
        self.key.clone().or(self.default_key())
    }
}

// ---------------------------------------------------------------------------
// Panes
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq)]
struct AssemblyView {
    symbol: Symbol,
}

impl Component for AssemblyView {
    fn render(&self) -> impl IntoElement {
        let assembly = self.symbol.data.assembly(&self.symbol.object);

        let Some(assembly) = assembly else {
            return rect()
                .padding(5.0)
                .child(label().text("Assembly unavailable"));
        };

        let data = AsmData {
            assembly,
            object: self.symbol.object.clone(),
        };
        let length = data.assembly.instructions.len();

        rect()
            .width(Size::fill())
            .height(Size::fill())
            .padding(5.0)
            .background(ASM_PANE_BG)
            .child(
                VirtualScrollView::new_with_data(data, |i, data: &AsmData| {
                    InstructionRow {
                        data: data.clone(),
                        index: i,
                        key: DiffKey::None,
                    }
                    .key(data.assembly.instructions[i].address)
                    .into()
                })
                .length(length)
                .item_size(ROW_HEIGHT),
            )
    }
}

fn symbol_info(symbol: &Symbol) -> impl IntoElement {
    let data = &symbol.data;

    rect()
        .width(Size::fill())
        .child(info_line(format!("Symbol: `{}`", data.name)))
        .maybe_child(
            data.demangled
                .as_ref()
                .map(|demangled| info_line(format!("Demangled: `{}`", demangled))),
        )
        .maybe_child(
            data.section
                .as_ref()
                .map(|section| info_line(format!("Section: `{}`", section.name))),
        )
        .child(info_line(format!("Size: {} bytes", data.size)))
        .child(info_line(format!(
            "Data Length: `{:?}`",
            data.data().map(|d| d.len()).unwrap_or_default()
        )))
}

// ---------------------------------------------------------------------------
// Tabs
// ---------------------------------------------------------------------------

/// One of the four dockable views. A tab is a persistent view rather than a slot
/// the selection drives, so each one renders itself off the current `Selection`
/// and subscribes to the state it needs on its own -- which also keeps a
/// selection change from re-rendering the whole tree.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Tab {
    Objects,
    Symbols,
    Info,
    Assembly,
}

impl Tab {
    /// The label shown in the tab bar.
    fn title(self) -> &'static str {
        match self {
            Tab::Objects => "Objects",
            Tab::Symbols => "Symbols",
            Tab::Info => "Info",
            Tab::Assembly => "Assembly",
        }
    }

    fn view(self) -> Element {
        match self {
            Tab::Objects => ObjectsTab.into_element(),
            Tab::Symbols => SymbolsTab.into_element(),
            Tab::Info => InfoTab.into_element(),
            Tab::Assembly => AssemblyTab.into_element(),
        }
    }
}

#[derive(PartialEq)]
struct ObjectsTab;

impl Component for ObjectsTab {
    fn render(&self) -> impl IntoElement {
        let objects = use_consume::<Objects>().0;
        let current = use_consume::<Sel>().0.read().clone();

        let rows: Vec<Element> = objects
            .read()
            .iter()
            .map(|object| {
                ObjectRow {
                    object: object.clone(),
                    selected: matches!(&current, Selection::Object(selected) if Arc::ptr_eq(selected, object)),
                    key: DiffKey::None,
                }
                .key(Arc::as_ptr(object).addr())
                .into()
            })
            .collect();

        // The list used to sit in a flex column and grow without bound; a tab has
        // a height of its own, so it scrolls instead of being clipped.
        rect().expanded().background(Color::WHITE).child(
            ScrollView::new().child(rect().width(Size::fill()).children(rows).into_element()),
        )
    }
}

#[derive(PartialEq)]
struct SymbolsTab;

impl Component for SymbolsTab {
    fn render(&self) -> impl IntoElement {
        let symbols = use_consume::<Symbols>().0.read().clone();
        let selected = match &*use_consume::<Sel>().0.read() {
            Selection::Symbol(symbol) => Some(symbol.clone()),
            _ => None,
        };
        let symbol_count = symbols.0.len();

        rect().expanded().background(SYMBOL_PANE_BG).child(
            VirtualScrollView::new_with_data(
                (symbols, selected),
                |i, (symbols, selected): &(SymbolList, Option<Symbol>)| {
                    let symbol = &symbols.0[i];
                    SymbolRow {
                        symbols: symbols.clone(),
                        index: i,
                        selected: selected.as_ref() == Some(symbol),
                        key: DiffKey::None,
                    }
                    .key(Arc::as_ptr(&symbol.data).addr())
                    .into()
                },
            )
            .length(symbol_count)
            .item_size(ROW_HEIGHT),
        )
    }
}

#[derive(PartialEq)]
struct InfoTab;

impl Component for InfoTab {
    fn render(&self) -> impl IntoElement {
        let current = use_consume::<Sel>().0.read().clone();

        match &current {
            Selection::None => placeholder("Nothing selected"),
            Selection::Object(object) => rect()
                .expanded()
                .background(Color::WHITE)
                .child(info_line(format!("Object: `{}`", object.name)))
                .child(info_line(format!("Format: {:?}", object.format)))
                .child(info_line(format!("Symbols: {:?}", object.symbols.len())))
                .into(),
            Selection::Symbol(symbol) => rect()
                .expanded()
                .background(Color::WHITE)
                .child(ScrollView::new().child(symbol_info(symbol).into_element()))
                .into(),
        }
    }
}

#[derive(PartialEq)]
struct AssemblyTab;

impl Component for AssemblyTab {
    fn render(&self) -> impl IntoElement {
        let current = use_consume::<Sel>().0.read().clone();

        match &current {
            Selection::Symbol(symbol) => AssemblyView {
                symbol: symbol.clone(),
            }
            .into_element(),
            _ => placeholder("No symbol selected"),
        }
    }
}

// ---------------------------------------------------------------------------
// Docking
// ---------------------------------------------------------------------------

/// Panel ids are only ever looked up inside the area that handed them out, so
/// each area numbers its own panels from zero.
type PanelId = u32;

/// One docking area: the tree of splits and tabbed panels filling one of the two
/// resizable panes. The four tabs are shared between the two areas, so a drop
/// here has to take the tab out of `other` -- which is safe to write from
/// `on_drop` only because the two areas are separate `State`s, and freya's
/// docking holds a mutable borrow of just the one being dropped into.
///
/// Plain data apart from that handle, so the layout can be serialized later.
struct DockArea {
    tree: DockNode<Tab, PanelId>,
    next_panel_id: PanelId,
    other: Option<State<DockArea>>,
}

impl DockArea {
    /// An area split in two, one tab on top and a tabbed panel below, which is
    /// what the panes used to look like before they were dockable.
    fn stacked(top: Tab, bottom: Vec<Tab>) -> Self {
        Self {
            tree: DockNode::Split {
                direction: Direction::Vertical,
                children: vec![
                    DockNode::Panel(DockPanel::new(0, vec![top])),
                    DockNode::Panel(DockPanel::new(1, bottom)),
                ],
            },
            next_panel_id: 2,
            other: None,
        }
    }

    /// An area of one panel holding a single tab.
    fn single(tab: Tab) -> Self {
        Self {
            tree: DockNode::Panel(DockPanel::new(0, vec![tab])),
            next_panel_id: 1,
            other: None,
        }
    }

    fn take_panel_id(&mut self) -> PanelId {
        let panel_id = self.next_panel_id;
        self.next_panel_id += 1;
        panel_id
    }

    /// Whether `tab` is the one on top in whichever panel holds it.
    fn is_active(&self, tab: Tab) -> bool {
        let Some((panel_id, _)) = self.tree.find_tab(&tab) else {
            return false;
        };
        self.tree
            .panel(&panel_id)
            .and_then(|panel| panel.active_tab_id)
            == Some(tab)
    }

    /// Put `tab` into `panel_id` at `position`, or at the end when `None`, and
    /// take it out of every other panel of this area.
    fn place(&mut self, panel_id: PanelId, tab: Tab, position: Option<usize>) -> bool {
        let Some(panel) = self.tree.panel_mut(&panel_id) else {
            return false;
        };
        match position {
            Some(position) => panel.insert_tab(tab, position),
            None => panel.append_tab(tab),
        }
        self.tree.remove_tab_except(&tab, Some(&panel_id));
        true
    }

    /// Drop `tab`, which has just been dropped into the other area.
    fn evict(&mut self, tab: Tab) {
        if self.tree.remove_tab_except(&tab, None) {
            self.tidy();
        }
    }

    /// Fold away the panels a move emptied. An area that loses its last tab keeps
    /// one empty panel rather than going to `None`, so its pane stays on screen as
    /// a drop target and tabs can be dragged back into it.
    fn tidy(&mut self) {
        self.tree.close_empty_panels();
        if self.tree.is_empty() && !matches!(self.tree, DockNode::Panel(_)) {
            let panel_id = self.take_panel_id();
            self.tree = DockNode::Panel(DockPanel::new(panel_id, Vec::new()));
        }
    }
}

impl DockingModel for DockArea {
    type TabId = Tab;
    type PanelId = PanelId;
    type DropValue = Tab;

    fn root(&self) -> Option<&DockNode<Tab, PanelId>> {
        Some(&self.tree)
    }

    fn on_drop(&mut self, tab: Tab, target: DropTarget<PanelId>) -> bool {
        let dropped = match target {
            DropTarget::Tab { panel_id, position } => self.place(panel_id, tab, Some(position)),
            DropTarget::Center(panel_id) => self.place(panel_id, tab, None),
            DropTarget::Split { panel_id, side } => {
                let new_panel_id = self.next_panel_id;
                let new_panel = DockPanel::new(new_panel_id, vec![tab]);
                if self.tree.split_panel(&panel_id, side, &new_panel) {
                    self.next_panel_id += 1;
                    self.tree.remove_tab_except(&tab, Some(&new_panel_id));
                    true
                } else {
                    false
                }
            }
        };

        if dropped {
            self.tidy();
            // A drag carries only the tab, so the source area is not known -- but
            // there are only two, and dropping the tab where it already was is a
            // no-op for the other one.
            if let Some(mut other) = self.other {
                other.write().evict(tab);
            }
        }

        dropped
    }

    fn set_active(&mut self, panel_id: PanelId, tab: Tab) -> bool {
        let Some(panel) = self.tree.panel_mut(&panel_id) else {
            return false;
        };
        if !panel.tabs.contains(&tab) {
            return false;
        }
        panel.active_tab_id = Some(tab);
        true
    }
}

/// One tab header. The same shape the pane headers used to have, so a bar of them
/// reads like the old `HEADER_BG` strip.
fn tab_label(tab: Tab, background: Color) -> impl IntoElement {
    rect()
        .height(Size::px(ROW_HEIGHT))
        .horizontal()
        .cross_align(Alignment::Center)
        .padding(Gaps::new_symmetric(0.0, 8.0))
        .background(background)
        .border(right_hairline())
        .overflow(Overflow::Clip)
        .child(label().text(tab.title()).max_lines(1))
}

fn tab_header(ctx: TabContext<Tab>, area: State<DockArea>) -> Element {
    let background = if ctx.is_drop_target {
        SELECTED_BG
    } else if area.read().is_active(ctx.tab_id) {
        Color::WHITE
    } else {
        Color::TRANSPARENT
    };

    tab_label(ctx.tab_id, background).into_element()
}

/// The copy of the tab that follows the cursor while it is being dragged.
fn tab_drag(tab: Tab) -> Element {
    rect()
        .interactive(false)
        .border(right_hairline())
        .child(tab_label(tab, SELECTED_BG))
        .into_element()
}

fn tab_bar(ctx: TabBarContext<PanelId>) -> Element {
    rect()
        .width(Size::fill())
        .height(Size::px(ROW_HEIGHT))
        .horizontal()
        .background(HEADER_BG)
        .border(bottom_hairline())
        .children(ctx.tab_children)
        .into_element()
}

fn tab_content(tab: Option<Tab>) -> Element {
    match tab {
        Some(tab) => tab.view(),
        None => placeholder("Drag a tab here"),
    }
}

fn docking_area(area: State<DockArea>) -> impl IntoElement {
    DockingArea::new(
        area,
        |ctx: ContentContext<Tab, PanelId>| tab_content(ctx.tab_id),
        move |ctx: TabContext<Tab>| tab_header(ctx, area),
        tab_drag,
        tab_bar,
    )
    .preview_element(
        rect()
            .interactive(false)
            .expanded()
            .background(DROP_PREVIEW_BG),
    )
}

fn toolbar(objects: State<Vec<Arc<Object>>>) -> impl IntoElement {
    let on_open = move |_| {
        spawn(async move {
            let Some(handles) = AsyncFileDialog::new()
                .set_title("Open a binary file...")
                .pick_files()
                .await
            else {
                return;
            };

            let paths: Vec<PathBuf> = handles.iter().map(|h| h.path().to_path_buf()).collect();

            // Reading and parsing is CPU bound and can take seconds on a large
            // binary, so keep it off the UI thread and hand the result back.
            let (sender, receiver) = async_channel::bounded(1);
            std::thread::spawn(move || {
                let _ = sender.send_blocking(open_files(paths));
            });

            if let Ok(parsed) = receiver.recv().await {
                let mut objects = objects;
                objects.write().extend(parsed);
            }
        });
    };

    rect()
        .horizontal()
        .width(Size::fill())
        .border(bottom_hairline())
        .child(
            rect()
                .margin(4.0)
                .child(Button::new().on_press(on_open).child("Open")),
        )
}

/// Write the session out whenever it changes.
///
/// `use_side_effect` re-runs its callback whenever a `State` that was `read()` inside
/// it changes (`freya-core/src/lifecycle/effect.rs`), so reading the two state
/// contexts here makes this one observer the single choke point every mutation flows
/// through: the three `selection.set(..)` sites and the toolbar's `objects.write()`
/// know nothing about persistence, and neither will any future one. Step 3's history
/// push hangs off the same place -- it is the one spot that sees every selection.
fn use_save_on_change(objects: State<Vec<Arc<Object>>>, selection: State<Selection>) {
    // What this session has already written. It starts as the empty project the app
    // starts with -- deliberately *not* as the project `use_restore_on_startup` loaded.
    // The empty baseline means the state the app boots into (no objects, no selection)
    // is never written, so a run in which nothing is opened, or one whose restore finds
    // no readable binary, or one closed while parsing is still in flight, all leave the
    // file on disk untouched. Seeding the baseline from the loaded project instead would
    // invert that: the effect's first run would see the still-empty state differ from
    // the baseline and write an *empty* project over a good one. The cost of the empty
    // baseline is a single redundant, idempotent save once a restore lands.
    let mut last_saved = use_state(Project::default);

    use_side_effect(move || {
        // Reading these subscribes the effect to them: any change re-runs it.
        let project = Project::from_state(&objects.read(), &selection.read());

        // `peek`, not `read`: the effect must not subscribe to the state it writes.
        if project != *last_saved.peek() {
            // A small JSON write that swallows its own errors, so this neither panics
            // nor meaningfully blocks the UI.
            project.save();
            last_saved.set(project);
        }
    });
}

/// Reopen the previous session's binaries and selection, once, at startup.
///
/// `use_hook` runs its initializer on mount and never again, which is what makes this
/// happen exactly once; `spawn` is freya's own task spawner and is callable during
/// render (`use_future` is built out of the same two calls), so the reading and
/// parsing is off the UI thread from the first frame. Beyond that this is the
/// toolbar's `on_open` pattern verbatim -- CPU-bound `open_files` on a `std::thread`,
/// the result back over an `async_channel` -- so a large binary parses with the window
/// already up and interactive.
///
/// Every step degrades silently: no state file or a corrupt one is `None`, a path that
/// no longer exists or no longer parses just contributes no `Object` (`open_files`
/// swallows its own failures), and `Project::resolve` falls back from a vanished symbol
/// to its object and from a vanished object to nothing.
fn use_restore_on_startup(objects: State<Vec<Arc<Object>>>, selection: State<Selection>) {
    use_hook(move || {
        let Some(project) = Project::load() else {
            return;
        };
        if project.binaries.is_empty() {
            return;
        }

        spawn(async move {
            let (sender, receiver) = async_channel::bounded(1);
            let paths = project.binaries.clone();
            std::thread::spawn(move || {
                let _ = sender.send_blocking(open_files(paths));
            });

            let Ok(parsed) = receiver.recv().await else {
                return;
            };
            // Nothing opened: leave the app empty *and* leave the file alone, so a
            // binary that is only temporarily missing is not forgotten.
            if parsed.is_empty() {
                return;
            }

            let (mut objects, mut selection) = (objects, selection);
            objects.write().extend(parsed);
            // Resolved against everything now loaded rather than just `parsed`, so
            // this stays correct if the user managed to open something first.
            let restored = project.resolve(&objects.read());
            selection.set(restored);
        });
    });
}

pub fn app() -> impl IntoElement {
    // The tooltip is a freya component whose theme hardcodes a 14px font, and the
    // theme is the only way in: its `theme` override field is private. floem's
    // tooltip was a plain label that inherited the interface font, so hand the theme
    // the interface size; the family it does inherit from the row it is attached to.
    use_init_theme(|| {
        let mut theme = Theme::default();

        if let Some(tooltip) = theme.get::<TooltipThemePreference>("tooltip").cloned() {
            theme.set(
                "tooltip",
                TooltipThemePreference {
                    font_size: Preference::Specific(fonts().ui.size),
                    ..tooltip
                },
            );
        }

        theme
    });

    let objects = use_provide_context(|| Objects(State::create(Vec::new()))).0;
    let selection = use_provide_context(|| Sel(State::create(Selection::None))).0;
    use_save_on_change(objects, selection);
    // After the save effect on purpose: the effect is in place, with its empty
    // baseline, before the restore can put anything into either state, so the
    // restored session is seen by it as an ordinary change.
    use_restore_on_startup(objects, selection);

    // Rebuilt only when the object list changes, not on every selection change.
    let symbols = use_memo(move || {
        SymbolList(Arc::new(
            objects
                .read()
                .iter()
                .flat_map(|object| {
                    object.symbols_sorted.iter().cloned().map(|data| Symbol {
                        object: object.clone(),
                        data,
                    })
                })
                .collect::<Vec<_>>(),
        ))
    });
    use_provide_context(move || Symbols(symbols));

    // One docking area per resizable pane: Objects over Symbols on the left, with
    // Info tabbed beside Symbols, and Assembly alone on the right. All four tabs
    // share one `DockDrag<Tab>`, which `use_drag` keeps at the root, so a tab can
    // be dragged from either area into the other; each area is told about the
    // other so the one taking a tab can evict it from the one losing it.
    let sidebar_dock = use_state(|| DockArea::stacked(Tab::Objects, vec![Tab::Symbols, Tab::Info]));
    let content_dock = use_state(|| DockArea::single(Tab::Assembly));
    use_hook(move || {
        let (mut sidebar_dock, mut content_dock) = (sidebar_dock, content_dock);
        sidebar_dock.write().other = Some(content_dock);
        content_dock.write().other = Some(sidebar_dock);
    });

    // The split is freya's own `ResizableContainer`: the sidebar panel keeps the
    // original fixed 300px (`PanelSize::px`, so the initial width is unchanged) and
    // the content panel is the single proportional one, which makes it take whatever
    // is left over -- the same thing the old `Size::flex(1.0)` did. Between them
    // freya inserts a `ResizableHandle`, a 4px draggable divider that replaces the
    // hairline border the sidebar used to draw. Docking cannot express a pixel
    // width, which is why this outer split is not itself a `DockingArea`.
    let split = ResizableContainer::new()
        .direction(Direction::Horizontal)
        .panel(
            ResizablePanel::new(PanelSize::px(300.0))
                .min_size(120.0)
                .child(docking_area(sidebar_dock)),
        )
        .panel(
            ResizablePanel::new(PanelSize::percent(100.0))
                .min_size(10.0)
                .child(docking_area(content_dock)),
        );

    rect()
        .expanded()
        .content(Content::Flex)
        .interface_font()
        .background(Color::WHITE)
        .child(toolbar(objects))
        // `ResizableContainer` renders itself `.expanded()`, so it needs a parent
        // that has already been given the leftover height under the toolbar.
        .child(
            rect()
                .width(Size::fill())
                .height(Size::flex(1.0))
                .child(split),
        )
}
