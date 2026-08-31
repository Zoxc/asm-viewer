//! Font settings read from KDE at runtime, so the interface matches the rest of the
//! desktop and the assembly view matches KWrite.

use std::{process::Command, sync::OnceLock};

pub struct Fonts {
    pub ui_family: String,
    pub ui_size: f32,
    pub mono_family: String,
    pub mono_size: f32,
}

/// KDE stores font sizes in points, freya wants logical pixels.
fn points_to_pixels(points: f32) -> f32 {
    points * 96.0 / 72.0
}

/// Font specs look like `Noto Sans Mono,10,-1,5,50,0,0,0,0,0`; only the family and
/// the point size are of interest here.
fn parse_spec(spec: &str) -> Option<(String, f32)> {
    let mut parts = spec.split(',');
    let family = parts.next()?.trim();
    let points: f32 = parts.next()?.trim().parse().ok()?;

    (!family.is_empty() && points > 0.0).then(|| (family.to_owned(), points_to_pixels(points)))
}

/// Ask KDE for a key in the `[General]` group of `kdeglobals`. Going through
/// `kreadconfig` rather than reading the file directly matters: neither `font` nor
/// `fixed` is written out until it is changed in System Settings, and only KDE knows
/// what its own defaults are.
fn query(key: &str) -> Option<(String, f32)> {
    ["kreadconfig6", "kreadconfig5"]
        .into_iter()
        .find_map(|bin| {
            let output = Command::new(bin)
                .args(["--group", "General", "--key", key])
                .output()
                .ok()?;

            if !output.status.success() {
                return None;
            }

            parse_spec(String::from_utf8(output.stdout).ok()?.trim())
        })
}

/// `font` is the general interface font and `fixed` the fixed-width one. Off a KDE
/// desktop there is nothing to ask, so fall back to the generic families, which
/// fontconfig resolves to whatever the system considers appropriate.
fn load() -> Fonts {
    let (ui_family, ui_size) =
        query("font").unwrap_or_else(|| ("sans-serif".to_owned(), points_to_pixels(10.0)));
    let (mono_family, mono_size) =
        query("fixed").unwrap_or_else(|| ("monospace".to_owned(), points_to_pixels(10.0)));

    Fonts {
        ui_family,
        ui_size,
        mono_family,
        mono_size,
    }
}

pub fn fonts() -> &'static Fonts {
    static FONTS: OnceLock<Fonts> = OnceLock::new();
    FONTS.get_or_init(load)
}

