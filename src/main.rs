#![feature(strict_provenance)]

use std::{fmt::Display, fs, path::PathBuf, sync::Arc};

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
use object::{read::archive::ArchiveFile, BinaryFormat, Object as _, ObjectSymbol, SymbolKind};
use symbolic_demangle::{Demangle, DemangleOptions};

struct Object {
    path: PathBuf,
    name: String,
    format: BinaryFormat,
    symbols: Vec<Arc<Symbol>>,
}

#[derive(Debug)]
struct Symbol {
    name: String,
    demangled: Option<String>,
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
            let symbols = file
                .symbols()
                .filter_map(|symbol| {
                    // Filter out non-text symbols
                    (symbol.kind() == SymbolKind::Text).then(|| ())?;

                    let name = String::from_utf8_lossy(symbol.name_bytes().ok()?).into_owned();
                    let demangled =
                        symbolic_common::Name::from(&name).demangle(DemangleOptions::complete());

                    Some(Arc::new(Symbol { name, demangled }))
                })
                .collect();

            objects.update(|list| {
                list.objects.push(Arc::new(Object {
                    name,
                    path,
                    format: file.format(),
                    symbols,
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
            ))
            .style(|s| s.flex_col());

            let data =
                stack((header("Symbol Info"), scroll(info))).style(|s| s.flex_col().width_full());
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
