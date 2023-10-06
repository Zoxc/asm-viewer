#![feature(strict_provenance)]

use std::{collections::HashMap, fmt::Display, fs, path::PathBuf, sync::Arc};

use floem::{
    event::Event,
    peniko::Color,
    reactive::{create_rw_signal, RwSignal},
    view::View,
    views::{
        container, container_box, dyn_container, empty, label, list, scroll, stack, text,
        virtual_list, Decorators, Label, VirtualListDirection, VirtualListItemSize,
    },
};
use iced_x86::Formatter;
use object::{
    read::archive::ArchiveFile, BinaryFormat, Object as _, ObjectSection, ObjectSymbol,
    SectionIndex, SymbolKind,
};
use symbolic_demangle::{Demangle, DemangleOptions};

struct Object {
    path: PathBuf,
    name: String,
    format: BinaryFormat,
    symbols: Vec<Arc<Symbol>>,
    sections: Vec<Arc<Section>>,
}

#[derive(Debug)]
struct Section {
    name: String,
    data: Vec<u8>,
    address: u64,

    // A sorted list of symbol positions
    symbols: Vec<u64>,
}

#[derive(Debug)]
struct Symbol {
    name: String,
    demangled: Option<String>,
    address: u64,
    section: Option<Arc<Section>>,
    size: u64,
}

impl Symbol {
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

    fn assembly(&self) -> Option<Arc<Assembly>> {
        let bytes = self.data()?;
        let mut decoder =
            iced_x86::Decoder::with_ip(64, bytes, self.address, iced_x86::DecoderOptions::NONE);

        let mut formatter = iced_x86::NasmFormatter::new();

        formatter.options_mut().set_digit_separator("`");
        formatter.options_mut().set_first_operand_char_index(10);

        let mut output = String::new();

        let mut instruction = iced_x86::Instruction::default();

        let mut assembly = Assembly {
            instructions: Vec::new(),
        };

        while decoder.can_decode() {
            decoder.decode_out(&mut instruction);

            output.clear();
            formatter.format(&instruction, &mut output);

            let start_index = (instruction.ip() - self.address) as usize;

            assembly.instructions.push(Instruction {
                address: instruction.ip(),
                bytes: bytes[start_index..start_index + instruction.len()].to_vec(),
                format: output.clone(),
            });

            // Eg. "00007FFAC46ACDB2 488DAC2400FFFFFF     lea       rbp,[rsp-100h]"
            let instr_bytes = &bytes[start_index..start_index + instruction.len()];
            /*for b in instr_bytes.iter() {
                print!("{:02X}", b);
            }
            if instr_bytes.len() < HEXBYTES_COLUMN_BYTE_LENGTH {
                for _ in 0..HEXBYTES_COLUMN_BYTE_LENGTH - instr_bytes.len() {
                    print!("  ");
                }
            }*/
        }

        Some(Arc::new(assembly))
    }
}

#[derive(Clone)]
struct Instruction {
    address: u64,
    bytes: Vec<u8>,
    format: String,
}

struct Assembly {
    instructions: Vec<Instruction>,
}

#[derive(Clone)]
enum Selection {
    None,
    Object(Arc<Object>),
    Symbol(Arc<Symbol>),
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
                    Some((
                        section.index(),
                        Section {
                            name,
                            address: section.address(),
                            data,
                            symbols: Vec::new(),
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

            let symbols = file
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

                    Some(Arc::new(Symbol {
                        name,
                        demangled,
                        section,
                        address: symbol.address(),
                        size: symbol.size(),
                    }))
                })
                .collect();

            objects.update(|list| {
                list.objects.push(Arc::new(Object {
                    name,
                    path,
                    format: file.format(),
                    symbols,
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

fn assembly(symbol: Arc<Symbol>) -> Box<dyn View> {
    if let Some(assembly) = symbol.assembly() {
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
            move |o| {
                text(o.format).style(|s| s.font_family("Consolas".to_string()).font_size(16.0))
            },
        )
        .style(|s| {
            s.flex_col()
                .background(Color::LIGHT_SLATE_GRAY)
                .width_full()
        });

        let instr = scroll(instr).style(|s| s.width_full().height_full());

        Box::new(instr)
    } else {
        Box::new(text("Assembly unavailable"))
    }
}

fn main_container(selection: Selection) -> Box<dyn View> {
    match selection {
        Selection::None => Box::new(text("Nothing selected").style(|s| s.padding(5.0))),
        Selection::Object(o) => {
            let data = stack((
                header("Object Info"),
                text(format!("Object: `{}`", o.name)).style(|s| s.padding(5.0)),
                text(format!("Format: {:?}", o.format)).style(|s| s.padding(5.0)),
            ))
            .style(|s| s.flex_col().width_full());
            Box::new(data)
        }
        Selection::Symbol(o) => {
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
                assembly(o),
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
                    .flat_map(|o| o.symbols.iter().cloned())
                    .collect::<im::Vector<_>>()
            })
        },
        |o| Arc::as_ptr(o).addr(),
        move |o| {
            let o_ = o.clone();
            text(o.demangled.as_ref().unwrap_or(&o.name).clone())
                .style(move |mut s| {
                    if selection.with(|s| {
                        if let Selection::Symbol(so) = s {
                            Arc::ptr_eq(so, &o_)
                        } else {
                            false
                        }
                    }) {
                        s = s.background(Color::LIGHT_GRAY);
                    }
                    s.padding(5).width_full().height_full()
                })
                .hover_style(|s| s.background(Color::LIGHT_GREEN))
                .on_click(move |_| {
                    selection.set(Selection::Symbol(o.clone()));
                    true
                })
        },
    )
    .style(|s| {
        s.flex_col()
            .background(Color::LIGHT_GOLDENROD_YELLOW)
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

    let content = dyn_container(move || selection.with(|s| s.clone()), main_container)
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
        .style(|s| s.flex_col().width_full().height_full().font_size(12.0))
        .window_title(|| "Assembly Viewer".to_string())
}

fn main() {
    floem::launch(app_view);
}
