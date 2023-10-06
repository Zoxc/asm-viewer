use std::{fmt::Display, path::PathBuf};

use floem::{
    event::Event,
    id::Id,
    peniko::Color,
    reactive::{create_rw_signal, create_signal, RwSignal},
    unit::UnitExt,
    view::View,
    views::{label, list, stack, text, Decorators, Label},
};

struct Object {
    id: Id,
    path: PathBuf,
}

struct ObjectList {
    objects: Vec<RwSignal<Object>>,
}

fn open_file(objects: RwSignal<ObjectList>) {
    let files = rfd::FileDialog::new()
        .set_title("Open a text file...")
        .pick_files();

    files.map(|files| {
        for path in files {
            objects.update(|list| {
                list.objects.push(create_rw_signal(Object {
                    id: Id::next(),
                    path,
                }))
            });
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

fn app_view() -> impl View {
    let objects = create_rw_signal(ObjectList {
        objects: Vec::new(),
    });

    let list = list(
        move || objects.with(|objects| objects.objects.clone()),
        |o| o.with(|o| o.id),
        |o| {
            text(o.with(|o| {
                o.path
                    .file_name()
                    .map(|name| name.to_string_lossy())
                    .unwrap_or_default()
                    .into_owned()
            }))
            .style(|s| s.padding(5))
            .hover_style(|s| s.background(Color::LIGHT_GREEN))
        },
    )
    .style(|s| s.flex_col().height_full());

    let object_list = stack((
        text("Objects").style(|s| s.padding(5.0).background(Color::WHITE_SMOKE).width_full()),
        list,
    ))
    .style(|s| {
        s.flex_col()
            .width(200)
            .height_full()
            .border_right(0.5)
            .border_color(Color::LIGHT_GRAY)
    });

    let content = text("Content").style(|s| s.width_full().height_full().background(Color::WHITE));

    let lower = stack((object_list, content)).style(|s| {
        s.flex_row()
            .items_start()
            .justify_start()
            .width_full()
            .height_full()
    });

    let bar = stack((
        button("Open", move |_| {
            open_file(objects);
            true
        }),
        button("Open2", move |_| {
            open_file(objects);
            true
        }),
    ))
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
