//! An icon drawn on whole device pixels: rasterized once per name, size and colour, and
//! drawn unscaled at a rounded place.
//!
//! **Not `SvgViewer`.** Its `image` draws the raster into the box at the box's fractional
//! place, and centring leaves a box on a half pixel whenever the room around it is odd. A
//! filter blends every pixel there with its neighbour. Nearest sampling looked like the
//! fix and is not one: at exactly half a pixel each pixel's centre lies on the edge
//! between two texels. The CPU rasterizer settles that the same way for every row, but the
//! window's GPU settles it row by row, so rows were doubled and dropped. Rounding the place
//! leaves nothing to sample (`notes/upstream/freya.md`).

use std::any::Any;
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;

use freya::engine::prelude::{raster_n32_premul, svg, FontMgr, SkImage};
use freya_core::data::{AccessibilityData, EffectData, LayoutData, StyleState, TextStyleData};
use freya_core::element::{
    ClipContext, ElementExt, EventHandlerType, LayoutContext, RenderContext,
};
use freya_core::elements::image::{Image, ImageElement, ImageHandle};
use freya_core::events::name::EventName;
use freya_core::integration::{DiffModifies, FxHashMap};

use super::*;

/// A Lucide icon, `side` logical pixels square, in `colour`.
#[derive(PartialEq)]
pub(crate) struct Glyph {
    pub(crate) icon: (&'static str, Bytes),
    pub(crate) side: f32,
    pub(crate) colour: Color,
}

impl Component for Glyph {
    fn render(&self) -> impl IntoElement {
        let scale = *Platform::get().scale_factor.read() as f32;
        let pixels = (self.side * scale).round().max(1.0) as u32;
        let raster = raster(self.icon.0, &self.icon.1, pixels, self.colour);
        let element = image(ImageHandle::new(raster, Bytes::new()))
            .width(Size::px(self.side))
            .height(Size::px(self.side))
            .into_element();
        let Element::Element {
            key,
            element,
            elements,
        } = element
        else {
            unreachable!("`image` builds an element");
        };
        let image = Image::try_downcast(element.as_ref()).expect("`image` builds an image");
        Element::Element {
            key,
            element: Rc::new(Snapped(image)),
            elements,
        }
    }
}

thread_local! {
    /// Every raster made, by the icon's name, its side in device pixels and its colour.
    /// A few dozen at most: the icons are the app's own, and so are their colours.
    static RASTERS: RefCell<HashMap<(&'static str, u32, Color), SkImage>> =
        RefCell::new(HashMap::new());
}

/// `bytes` rasterized `pixels` square in `colour`, made once per key. An icon that will not
/// parse is an empty raster: the SVGs are compiled in, so that is a bug and not an input.
pub(super) fn raster(name: &'static str, bytes: &[u8], pixels: u32, colour: Color) -> SkImage {
    RASTERS.with(|rasters| {
        rasters
            .borrow_mut()
            .entry((name, pixels, colour))
            .or_insert_with(|| {
                let side = pixels as i32;
                let mut surface =
                    raster_n32_premul((side, side)).expect("a raster surface of an icon's size");
                if let Ok(mut dom) = svg::Dom::from_bytes(bytes, FontMgr::empty()) {
                    dom.set_container_size((side, side));
                    let mut root = dom.root();
                    root.set_width(svg::Length::new(pixels as f32, svg::LengthUnit::PX));
                    root.set_height(svg::Length::new(pixels as f32, svg::LengthUnit::PX));
                    root.set_color(colour.into());
                    dom.render(surface.canvas());
                }
                surface.image_snapshot()
            })
            .clone()
    })
}

/// freya's `ImageElement` in every respect but how it draws: the raster at its own size,
/// centred in the box and put on whole device pixels.
struct Snapped(ImageElement);

impl Snapped {
    fn of(other: &Rc<dyn ElementExt>) -> Option<&ImageElement> {
        (other.as_ref() as &dyn Any)
            .downcast_ref::<Snapped>()
            .map(|snapped| &snapped.0)
    }
}

/// Whether an element is a glyph: how a test finds the icons it counts or presses.
#[cfg(test)]
pub(crate) fn is_glyph(element: &dyn ElementExt) -> bool {
    (element as &dyn Any).is::<Snapped>()
}

impl ElementExt for Snapped {
    fn changed(&self, other: &Rc<dyn ElementExt>) -> bool {
        Self::of(other).is_some_and(|other| &self.0 != other)
    }

    fn diff(&self, other: &Rc<dyn ElementExt>) -> DiffModifies {
        match Self::of(other) {
            Some(other) => self.0.diff(&(Rc::new(other.clone()) as Rc<dyn ElementExt>)),
            None => DiffModifies::all(),
        }
    }

    fn layout(&'_ self) -> Cow<'_, LayoutData> {
        self.0.layout()
    }

    fn effect(&'_ self) -> Option<Cow<'_, EffectData>> {
        self.0.effect()
    }

    fn style(&'_ self) -> Cow<'_, StyleState> {
        self.0.style()
    }

    fn text_style(&'_ self) -> Cow<'_, TextStyleData> {
        self.0.text_style()
    }

    fn accessibility(&'_ self) -> Cow<'_, AccessibilityData> {
        self.0.accessibility()
    }

    fn layer(&self) -> Layer {
        self.0.layer()
    }

    fn events_handlers(&'_ self) -> Option<Cow<'_, FxHashMap<EventName, EventHandlerType>>> {
        self.0.events_handlers()
    }

    fn should_measure_inner_children(&self) -> bool {
        self.0.should_measure_inner_children()
    }

    fn should_hook_measurement(&self) -> bool {
        self.0.should_hook_measurement()
    }

    fn measure(&self, context: LayoutContext) -> Option<(Size2D, Rc<dyn Any>)> {
        self.0.measure(context)
    }

    fn clip(&self, context: ClipContext) {
        self.0.clip(context)
    }

    fn render(&self, context: RenderContext) {
        let raster = &self.0.image_handle.image;
        let area = context.layout_node.visible_area();
        let left = (area.center().x - raster.width() as f32 / 2.0).round();
        let top = (area.center().y - raster.height() as f32 / 2.0).round();
        context.canvas.draw_image(raster, (left, top), None);
    }
}
