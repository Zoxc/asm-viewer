use std::{path::PathBuf, sync::Arc};

use freya::prelude::*;
use rfd::AsyncFileDialog;

use crate::object::{open_files, Assembly, Object, Symbol, SymbolData};

/// Height of every row in the object, symbol and instruction lists. This must stay
/// equal to the `item_size` given to each `VirtualScrollView`.
const ROW_HEIGHT: f32 = 26.0;
const ASM_FONT: &str = "Consolas";
const ASM_FONT_SIZE: f32 = 14.0;

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

#[derive(Clone)]
pub enum Selection {
    None,
    Object(Arc<Object>),
    Symbol(Symbol),
}

impl PartialEq for Selection {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Selection::None, Selection::None) => true,
            (Selection::Object(a), Selection::Object(b)) => Arc::ptr_eq(a, b),
            (Selection::Symbol(a), Selection::Symbol(b)) => a == b,
            _ => false,
        }
    }
}

/// The loaded objects, shared through context.
#[derive(Clone, Copy)]
struct Objects(State<Vec<Arc<Object>>>);

/// The current selection, shared through context.
#[derive(Clone, Copy)]
struct Sel(State<Selection>);

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

fn header(title: &'static str) -> impl IntoElement {
    rect()
        .width(Size::fill())
        .padding(5.0)
        .background(HEADER_BG)
        .border(bottom_hairline())
        .child(label().text(title))
}

fn info_line(text: String) -> impl IntoElement {
    rect().padding(5.0).child(label().text(text))
}

fn kind_color(kind: iced_x86::FormatterTextKind) -> Color {
    match kind {
        iced_x86::FormatterTextKind::Mnemonic | iced_x86::FormatterTextKind::Prefix => MNEMONIC_FG,
        iced_x86::FormatterTextKind::Register => REGISTER_FG,
        iced_x86::FormatterTextKind::Number => NUMBER_FG,
        _ => OTHER_FG,
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
            .child(
                label()
                    .text(text)
                    .max_lines(1)
                    .color(if hovering() { RELOC_HOVER_FG } else { RELOC_FG }),
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

        let spans = instruction.format.iter().map(|(text, kind)| {
            Span::new(text.clone())
                .color(kind_color(*kind))
                .font_family(ASM_FONT)
                .font_size(ASM_FONT_SIZE)
                .font_weight(if *kind == iced_x86::FormatterTextKind::Mnemonic {
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
            .font_family(ASM_FONT)
            .font_size(ASM_FONT_SIZE)
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
            .height(Size::flex(1.0))
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

fn main_container(selection: &Selection) -> Element {
    match selection {
        Selection::None => rect()
            .padding(5.0)
            .child(label().text("Nothing selected"))
            .into(),
        Selection::Object(object) => rect()
            .width(Size::fill())
            .child(header("Object Info"))
            .child(info_line(format!("Object: `{}`", object.name)))
            .child(info_line(format!("Format: {:?}", object.format)))
            .child(info_line(format!("Symbols: {:?}", object.symbols.len())))
            .into(),
        Selection::Symbol(symbol) => rect()
            .width(Size::fill())
            .height(Size::fill())
            .content(Content::Flex)
            .child(header("Symbol Info"))
            .child(ScrollView::new().height(Size::auto()).child(
                symbol_info(symbol).into_element(),
            ))
            .child(header("Assembly"))
            .child(AssemblyView {
                symbol: symbol.clone(),
            })
            .into(),
    }
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

pub fn app() -> impl IntoElement {
    let objects = use_provide_context(|| Objects(State::create(Vec::new()))).0;
    let selection = use_provide_context(|| Sel(State::create(Selection::None))).0;

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

    let current = selection.read().clone();
    let symbols = symbols.read().clone();
    let symbol_count = symbols.0.len();

    let object_rows: Vec<Element> = objects
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

    let selected_symbol = match &current {
        Selection::Symbol(symbol) => Some(symbol.clone()),
        _ => None,
    };

    let sidebar = rect()
        .width(Size::px(300.0))
        .height(Size::fill())
        .content(Content::Flex)
        .background(Color::WHITE)
        .border(right_hairline())
        .child(header("Objects"))
        .child(rect().width(Size::fill()).children(object_rows))
        .child(header("Symbols"))
        .child(
            rect()
                .width(Size::fill())
                .height(Size::flex(1.0))
                .background(SYMBOL_PANE_BG)
                .child(
                    VirtualScrollView::new_with_data(
                        (symbols, selected_symbol),
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
                ),
        );

    let content = rect()
        .width(Size::flex(1.0))
        .height(Size::fill())
        .background(Color::WHITE)
        .child(main_container(&current));

    rect()
        .expanded()
        .content(Content::Flex)
        .font_size(12.0)
        .background(Color::WHITE)
        .child(toolbar(objects))
        .child(
            rect()
                .horizontal()
                .content(Content::Flex)
                .width(Size::fill())
                .height(Size::flex(1.0))
                .child(sidebar)
                .child(content),
        )
}
