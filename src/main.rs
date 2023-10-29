#![feature(strict_provenance)]

use std::{collections::HashMap, fmt::Display, fs, ops::Range, path::PathBuf, sync::Arc};

use floem::{
    cosmic_text::{Attrs, AttrsList, FamilyOwned, Style, TextLayout, Weight},
    event::Event,
    peniko::Color,
    reactive::{create_rw_signal, RwSignal},
    style::{CursorStyle, TextOverflow},
    view::View,
    views::{
        bg_active_color, container, container_box, dyn_container, empty, label, list, rich_text,
        scroll, stack, text, virtual_list, Decorators, Label, VirtualListDirection,
        VirtualListItemSize,
    },
};
use iced_x86::Formatter;
use object::{
    read::archive::ArchiveFile, BinaryFormat, Object as _, ObjectSection, ObjectSymbol, Relocation,
    RelocationTarget, SectionIndex, SymbolIndex, SymbolKind,
};
use symbolic_demangle::{Demangle, DemangleOptions};

struct Object {
    path: PathBuf,
    name: String,
    format: BinaryFormat,
    symbols: HashMap<SymbolIndex, Arc<SymbolData>>,
    symbols_sorted: Vec<Arc<SymbolData>>,
    sections: Vec<Arc<Section>>,
}

#[derive(Debug)]
struct Section {
    name: String,
    data: Vec<u8>,
    address: u64,

    relocations: HashMap<u64, Relocation>,

    // A sorted list of symbol positions
    symbols: Vec<u64>,
}

#[derive(Debug)]
struct SymbolData {
    name: String,
    demangled: Option<String>,
    address: u64,
    section: Option<Arc<Section>>,
    size: u64,
}

impl SymbolData {
    fn estimate_size(&self) -> Option<u64> {
        let section = self.section.as_ref()?;
        let i = section.symbols.binary_search(&self.address).ok()?;
        if i + 1 == section.symbols.len() {
            section
                .address
                .checked_add(section.data.len().try_into().ok()?)?
                .checked_sub(self.address)
        } else {
            section.symbols[i + 1].checked_sub(self.address)
        }
    }

    fn data(&self) -> Option<&[u8]> {
        let section = self.section.as_ref()?;
        let size: usize = self.estimate_size()?.try_into().ok()?;
        let offset: usize = self.address.checked_sub(section.address)?.try_into().ok()?;
        let end = offset.checked_add(size)?;
        section.data.get(offset..end)
    }

    fn assembly(&self, object: &Object) -> Option<Arc<Assembly>> {
        let bytes = self.data()?;
        let mut decoder =
            iced_x86::Decoder::with_ip(64, bytes, self.address, iced_x86::DecoderOptions::NONE);

        let mut formatter = iced_x86::IntelFormatter::new();

        formatter.options_mut().set_first_operand_char_index(10);
        formatter
            .options_mut()
            .set_space_after_operand_separator(true);

        let mut instruction = iced_x86::Instruction::default();

        let mut assembly = Assembly {
            instructions: Vec::new(),
        };

        while decoder.can_decode() {
            decoder.decode_out(&mut instruction);

            let start_index = (instruction.ip() - self.address) as usize;

            let mut relocation = None;

            self.section.as_ref().map(|section| {
                for i in 0..instruction.len() {
                    section
                        .relocations
                        .get(&(instruction.ip() + i as u64))
                        .map(|r| {
                            relocation = Some(r.target().clone());
                        });
                }
            });

            let relocation = relocation.and_then(|r| match r {
                RelocationTarget::Symbol(i) => object.symbols.get(&i).cloned(),
                _ => None,
            });

            let mut inst = Instruction {
                address: instruction.ip(),
                bytes: bytes[start_index..start_index + instruction.len()].to_vec(),
                format: Vec::new(),
                relocation,
            };
            formatter.format(&instruction, &mut inst);

            assembly.instructions.push(inst);
        }

        Some(Arc::new(assembly))
    }
}

#[derive(Clone)]
struct Symbol {
    object: Arc<Object>,
    data: Arc<SymbolData>,
}

#[derive(Clone)]
struct Instruction {
    address: u64,
    bytes: Vec<u8>,
    format: Vec<(String, iced_x86::FormatterTextKind)>,
    relocation: Option<Arc<SymbolData>>,
}

impl iced_x86::FormatterOutput for Instruction {
    fn write(&mut self, text: &str, kind: iced_x86::FormatterTextKind) {
        self.format.push((text.to_owned(), kind));
    }

    fn write_number(
        &mut self,
        _instruction: &iced_x86::Instruction,
        _operand: u32,
        _instruction_operand: Option<u32>,
        text: &str,
        _value: u64,
        _number_kind: iced_x86::NumberKind,
        kind: iced_x86::FormatterTextKind,
    ) {
        if self.relocation.is_none() {
            self.write(text, kind);
        }
    }
}

struct Assembly {
    instructions: Vec<Instruction>,
}

#[derive(Clone)]
enum Selection {
    None,
    Object(Arc<Object>),
    Symbol(Symbol),
}

struct ObjectList {
    objects: Vec<Arc<Object>>,
}

fn open_object(objects: &RwSignal<ObjectList>, data: &[u8], name: String, path: PathBuf) {
    object::File::parse(data)
        .map(|file| {
            let mut sections: HashMap<SectionIndex, Section> = file
                .sections()
                .filter_map(|section| {
                    let name = String::from_utf8_lossy(section.name_bytes().ok()?).into_owned();
                    let data = section.uncompressed_data().ok()?.into_owned();
                    let relocations = section.relocations().collect();
                    Some((
                        section.index(),
                        Section {
                            name,
                            address: section.address(),
                            data,
                            symbols: Vec::new(),
                            relocations,
                        },
                    ))
                })
                .collect();

            // Insert symbol addresses into sections
            file.symbols().for_each(|symbol| {
                if symbol.kind() != SymbolKind::Text {
                    return;
                }

                symbol
                    .section()
                    .index()
                    .and_then(|index| sections.get_mut(&index))
                    .map(|section| section.symbols.push(symbol.address()));
            });

            let section_map: HashMap<SectionIndex, Arc<Section>> = sections
                .into_iter()
                .map(|(index, mut section)| {
                    section.symbols.sort_unstable();
                    (index, Arc::new(section))
                })
                .collect();

            let sections = section_map.values().cloned().collect();

            let symbols: HashMap<_, _> = file
                .symbols()
                .filter_map(|symbol| {
                    // Filter out non-text symbols
                    (symbol.kind() == SymbolKind::Text).then(|| ())?;

                    let name = String::from_utf8_lossy(symbol.name_bytes().ok()?).into_owned();
                    let demangled =
                        symbolic_common::Name::from(&name).demangle(DemangleOptions::complete());

                    let section = symbol
                        .section()
                        .index()
                        .and_then(|index| section_map.get(&index).cloned());

                    Some((
                        symbol.index(),
                        Arc::new(SymbolData {
                            name,
                            demangled,
                            section,
                            address: symbol.address(),
                            size: symbol.size(),
                        }),
                    ))
                })
                .collect();

            let mut symbols_sorted: Vec<_> = symbols.values().cloned().collect();
            symbols_sorted.sort_unstable_by(|a, b| a.name.cmp(&b.name));

            objects.update(|list| {
                list.objects.push(Arc::new(Object {
                    name,
                    path,
                    format: file.format(),
                    symbols,
                    symbols_sorted,
                    sections,
                }))
            });
        })
        .ok();
}

fn open_file(objects: RwSignal<ObjectList>) {
    let files = rfd::FileDialog::new()
        .set_title("Open a binary file...")
        .pick_files();

    files.map(|files| {
        for path in files {
            let file = fs::read(&path).unwrap();

            if let Ok(archive) = ArchiveFile::parse(file.as_slice()) {
                for member in archive.members() {
                    member
                        .map(|member| {
                            let name = String::from_utf8_lossy(member.name()).into_owned();
                            member
                                .data(file.as_slice())
                                .map(|data| {
                                    open_object(&objects, data, name, path.clone());
                                })
                                .ok();
                        })
                        .ok();
                }
            }

            open_object(
                &objects,
                file.as_slice(),
                path.file_name()
                    .map(|name| name.to_string_lossy())
                    .unwrap_or_default()
                    .into_owned(),
                path.clone(),
            );
        }
    });
}

fn button(label: impl Display, click: impl Fn(&Event) -> bool + 'static) -> Label {
    text(label)
        .style(|s| {
            s.border_radius(3.0)
                .padding(6.0)
                .background(Color::WHITE)
                // .box_shadow_blur(1.0)
                // .box_shadow_color(Color::GRAY)
                .border_color(Color::GRAY)
                .border(0.5)
                .margin(4)
        })
        .on_click(click)
        .hover_style(|s| s.background(Color::LIGHT_GREEN))
        .active_style(|s| s.color(Color::WHITE).background(Color::DARK_GREEN))
        .keyboard_navigatable()
        .focus_visible_style(|s| s.border_color(Color::BLUE).border(2.))
}

fn header(label: impl Display) -> Label {
    text(label).style(|s| {
        s.padding(5.0)
            .background(Color::WHITE_SMOKE)
            .width_full()
            .border_bottom(0.5)
            .border_color(Color::LIGHT_GRAY)
    })
}

fn assembly(symbol: Symbol, selection: RwSignal<Selection>) -> Box<dyn View> {
    if let Some(assembly) = symbol.data.assembly(&symbol.object) {
        let instr = virtual_list(
            VirtualListDirection::Vertical,
            VirtualListItemSize::Fixed(Box::new(|| 26.0)),
            move || {
                assembly
                    .instructions
                    .iter()
                    .cloned()
                    .collect::<im::Vector<_>>()
            },
            |i| i.address,
            move |i| {
                let address = text(format!("{:016X} ", i.address))
                    .style(|s| s.width(200).color(Color::rgb8(118, 141, 169)));

                let format: Vec<_> = i.format.iter().map(|(s, _)| &**s).collect();
                let format: String = format.join("");

                let family: Vec<FamilyOwned> = FamilyOwned::parse_list("Consolas").collect();
                let attrs = Attrs::new()
                    .color(Color::BLACK)
                    .font_size(14.0)
                    .family(&family);
                let mut attrs_list = AttrsList::new(attrs);
                let mut offset = 0;
                for (string, kind) in i.format {
                    let color = match kind {
                        iced_x86::FormatterTextKind::Mnemonic
                        | iced_x86::FormatterTextKind::Prefix => Color::rgb8(116, 94, 147),
                        iced_x86::FormatterTextKind::Register => Color::rgb8(87, 103, 65),
                        iced_x86::FormatterTextKind::Number => Color::rgb8(80, 107, 135),
                        _ => Color::rgb8(102, 102, 102),
                    };
                    attrs_list.add_span(
                        Range {
                            start: offset,
                            end: offset + string.len(),
                        },
                        Attrs::new()
                            .color(color)
                            .family(&family)
                            .font_size(14.0)
                            .weight(if kind == iced_x86::FormatterTextKind::Mnemonic {
                                Weight::BOLD
                            } else {
                                Weight::NORMAL
                            }),
                    );
                    offset += string.len();
                }
                let mut text_layout = TextLayout::new();
                text_layout.set_text(&format, attrs_list);

                let format = rich_text(move || text_layout.clone());
                let reloc = i
                    .relocation
                    .map(|s| {
                        let symbol = Symbol {
                            object: symbol.object.clone(),
                            data: s.clone(),
                        };
                        text(s.demangled.as_ref().unwrap_or(&s.name).clone()).on_click(move |_| {
                            selection.set(Selection::Symbol(symbol.clone()));
                            true
                        })
                    })
                    .unwrap_or_else(|| text(""));

                let reloc = reloc
                    .style(|s| {
                        s.cursor(CursorStyle::Pointer)
                            .color(Color::rgb8(50, 50, 50))
                    })
                    .hover_style(|s| {
                        s.color(Color::rgb8(105, 89, 132))
                            .border_radius(6)
                            .border_bottom(2)
                            .border_color(Color::rgb8(105, 89, 132))
                            .background(Color::WHITE.with_alpha_factor(0.6))
                    });

                //let bytes: Vec<String> = i.bytes.iter().map(|b| format!("{:02X} ", b)).collect();
                //let bytes = text(bytes.join(" ")).style(|s| s.width(200).color(Color::GRAY));
                stack((address, format, reloc))
                    .style(|s| {
                        s.font_family("Consolas".to_string())
                            .font_size(14.0)
                            .padding(3)
                            .height(26.0)
                    })
                    .hover_style(|s| s.background(Color::rgba8(228, 237, 216, 160)))
            },
        )
        .style(|s| s.flex_col().padding(5).width_full());

        let instr = scroll(instr).style(|s| {
            s.width_full()
                .height_full()
                .background(Color::rgb8(248, 248, 248))
        });

        Box::new(instr)
    } else {
        Box::new(text("Assembly unavailable").style(|s| s.padding(5.0)))
    }
}

fn main_container(current: Selection, selection: RwSignal<Selection>) -> Box<dyn View> {
    match current {
        Selection::None => Box::new(text("Nothing selected").style(|s| s.padding(5.0))),
        Selection::Object(o) => {
            let data = stack((
                header("Object Info"),
                text(format!("Object: `{}`", o.name)).style(|s| s.padding(5.0)),
                text(format!("Format: {:?}", o.format)).style(|s| s.padding(5.0)),
                text(format!("Symbols: {:?}", o.symbols.len())).style(|s| s.padding(5.0)),
            ))
            .style(|s| s.flex_col().width_full());
            Box::new(data)
        }
        Selection::Symbol(symbol) => {
            let o = &symbol.data;
            let info = stack((
                text(format!("Symbol: `{}`", o.name)).style(|s| s.padding(5.0)),
                o.demangled
                    .as_ref()
                    .map(|demangled| {
                        container_box(
                            text(format!("Demangled: `{}`", demangled)).style(|s| s.padding(5.0)),
                        )
                    })
                    .unwrap_or_else(|| container_box(empty())),
                o.section
                    .as_ref()
                    .map(|section| {
                        container_box(
                            text(format!("Section: `{}`", section.name)).style(|s| s.padding(5.0)),
                        )
                    })
                    .unwrap_or_else(|| container_box(empty())),
                text(format!("Size: {} bytes", o.size)).style(|s| s.padding(5.0)),
                text(format!(
                    "Data Length: `{:?}`",
                    o.data().map(|d| d.len()).unwrap_or_default()
                ))
                .style(|s| s.padding(5.0)),
            ))
            .style(|s| s.flex_col());

            let data = stack((
                header("Symbol Info"),
                scroll(info),
                header("Assembly"),
                assembly(symbol, selection),
            ))
            .style(|s| s.flex_col().width_full().height_full());
            Box::new(data)
        }
    }
}

fn app_view() -> impl View {
    let objects = create_rw_signal(ObjectList {
        objects: Vec::new(),
    });

    let selection = create_rw_signal(Selection::None);

    let object_list = list(
        move || objects.with(|objects| objects.objects.clone()),
        |o| Arc::as_ptr(o).addr(),
        move |o| {
            let o_ = o.clone();
            text(o.name.clone())
                .style(move |s| {
                    s.apply_if(
                        selection.with(|s| {
                            if let Selection::Object(so) = s {
                                Arc::ptr_eq(so, &o_)
                            } else {
                                false
                            }
                        }),
                        |s| s.background(Color::LIGHT_GRAY),
                    )
                    .padding(5)
                    .width_full()
                    .height(26.0)
                    .text_overflow(TextOverflow::Clip)
                })
                .hover_style(|s| s.background(Color::LIGHT_GREEN))
                .on_click(move |_| {
                    selection.set(Selection::Object(o.clone()));
                    true
                })
        },
    )
    .style(|s| s.flex_col().height_full());

    let symbol_list = virtual_list(
        VirtualListDirection::Vertical,
        VirtualListItemSize::Fixed(Box::new(|| 26.0)),
        move || {
            objects.with(|objects| {
                objects
                    .objects
                    .iter()
                    .flat_map(|o| {
                        o.symbols_sorted.iter().cloned().map(|s| Symbol {
                            object: o.clone(),
                            data: s,
                        })
                    })
                    .collect::<im::Vector<_>>()
            })
        },
        |o| Arc::as_ptr(&o.data).addr(),
        move |o| {
            let o_ = o.clone();
            text(o.data.demangled.as_ref().unwrap_or(&o.data.name).clone())
                .style(move |mut s| {
                    if selection.with(|s| {
                        if let Selection::Symbol(so) = s {
                            Arc::ptr_eq(&so.data, &o_.data)
                        } else {
                            false
                        }
                    }) {
                        s = s.background(Color::LIGHT_GRAY);
                    }
                    s.padding(5)
                        .width_full()
                        .height(26.0)
                        .text_overflow(TextOverflow::Clip)
                })
                .hover_style(|s| s.background(Color::rgb8(226, 226, 205)))
                .on_click(move |_| {
                    selection.set(Selection::Symbol(o.clone()));
                    true
                })
        },
    )
    .style(|s| {
        s.flex_col()
            .background(Color::rgb8(243, 243, 228))
            .width_full()
    });

    let symbol_list = scroll(symbol_list).style(|s| s.width_full().height_full());

    let object_list = stack((
        header("Objects"),
        object_list,
        header("Symbols"),
        symbol_list,
    ))
    .style(|s| {
        s.flex_col()
            .width(300)
            .height_full()
            .border_right(0.5)
            .border_color(Color::LIGHT_GRAY)
    });

    let content = dyn_container(
        move || selection.with(|s| s.clone()),
        move |current| main_container(current, selection),
    )
    .style(|s| s.width_full().height_full().background(Color::WHITE));

    let lower = stack((object_list, content)).style(|s| {
        s.flex_row()
            .items_start()
            .justify_start()
            .width_full()
            .height_full()
    });

    let bar = stack((button("Open", move |_| {
        open_file(objects);
        true
    }),))
    .style(|s| {
        s.flex_row()
            .items_start()
            .justify_start()
            .border_bottom(0.5)
            .border_color(Color::LIGHT_GRAY)
    });

    stack((bar, lower))
        .style(|s| {
            s.flex_col()
                .width_full()
                .height_full()
                .font_size(12.0)
                .scroll_bar_thickness(20.0)
                .scroll_bar_rounded(false)
                .scroll_bar_color(Color::rgba8(166, 166, 166, 140))
                .scroll_bar_drag_color(Color::rgb8(166, 166, 166))
                .scroll_bar_hover_color(Color::rgb8(184, 184, 184))
                .set(bg_active_color, Color::rgba8(166, 166, 166, 40))
        })
        .window_title(|| "Assembly Viewer".to_string())
}

fn main() {
    env_logger::init();
    floem::launch(app_view);
}
