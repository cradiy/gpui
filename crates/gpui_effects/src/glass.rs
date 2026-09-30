use gpui::{
    App, Bounds, Corners, Div, EffectUniforms, ElementId, Hsla, InteractiveElement, Interactivity,
    IntoElement, PaintBackdropEffect, ParentElement, Pixels, Point, RenderOnce, Rgba, Size,
    StyleRefinement, Styled, Window, div, hsla, px,
};

use crate::frosted_glass_shader;

const SHAPE_A_SLOT: usize = 0;
const SHAPE_B_SLOT: usize = 1;
const SHAPE_CONFIG_SLOT: usize = 2;
const TINT_SLOT: usize = 3;
const APPEARANCE_SLOT: usize = 4;
const EDGE_SLOT: usize = 5;
const SURFACE_SLOT: usize = 6;

/// One rounded region inside a mergeable frosted-glass field.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrostedGlassShape {
    pub center: Point<Pixels>,
    pub size: Size<Pixels>,
    pub radius: Pixels,
}

impl FrostedGlassShape {
    pub fn new(center: Point<Pixels>, size: Size<Pixels>, radius: Pixels) -> Self {
        Self {
            center,
            size,
            radius,
        }
    }
}

/// Material-specific configuration for [`FrostedGlass`].
///
/// Layout, foreground color, opacity, borders, shadows, and typography use the
/// normal [`Styled`] API on [`FrostedGlass`] itself.
#[derive(Clone, Copy, Debug, uic_macros::Chainable)]
pub struct FrostedGlassAppearance {
    pub blur_radius: Pixels,
    pub saturation: f32,
    pub brightness: f32,
    pub tint: Hsla,
    pub edge: Hsla,
    pub edge_width: Pixels,
    pub sheen: f32,
    pub merge_distance: Pixels,
}

impl FrostedGlassAppearance {
    pub fn dark() -> Self {
        Self {
            blur_radius: px(8.0),
            saturation: 1.10,
            brightness: 0.92,
            tint: hsla(0.61, 0.30, 0.12, 0.32),
            edge: hsla(0.58, 0.10, 0.98, 0.26),
            edge_width: px(1.0),
            sheen: 0.022,
            merge_distance: px(44.0),
        }
    }

    pub fn light() -> Self {
        Self {
            blur_radius: px(8.0),
            saturation: 1.03,
            brightness: 1.01,
            tint: hsla(0.58, 0.08, 0.98, 0.24),
            edge: hsla(0.58, 0.08, 1.0, 0.34),
            edge_width: px(1.0),
            sheen: 0.018,
            merge_distance: px(44.0),
        }
    }
}

impl Default for FrostedGlassAppearance {
    fn default() -> Self {
        Self::dark()
    }
}

/// A layout-neutral frosted-glass container.
///
/// With no explicit shapes, the glass fills its own bounds and uses the normal
/// GPUI corner-radius style. [`FrostedGlass::merge`] switches to a local field
/// containing two rounded shapes whose blur masks join as they approach.
/// Children are painted afterward and remain sharp.
pub struct FrostedGlass {
    div: Div,
    appearance: FrostedGlassAppearance,
    shapes: [Option<FrostedGlassShape>; 2],
}

impl FrostedGlass {
    pub fn new() -> Self {
        Self::with_appearance(FrostedGlassAppearance::default())
    }

    pub fn with_appearance(appearance: FrostedGlassAppearance) -> Self {
        Self {
            div: div(),
            appearance,
            shapes: [None, None],
        }
    }

    pub fn appearance(mut self, appearance: FrostedGlassAppearance) -> Self {
        self.appearance = appearance;
        self
    }

    /// Uses two local shapes and smoothly joins their frosted masks.
    pub fn merge(mut self, first: FrostedGlassShape, second: FrostedGlassShape) -> Self {
        self.shapes = [Some(first), Some(second)];
        self
    }

    /// Uses one explicit local shape instead of the element's full bounds.
    pub fn shape(mut self, shape: FrostedGlassShape) -> Self {
        self.shapes = [Some(shape), None];
        self
    }

    /// Sets an element id without wrapping this render-once component in
    /// `Stateful<FrostedGlass>`.
    pub fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.div.interactivity().element_id = Some(id.into());
        self
    }
}

impl Default for FrostedGlass {
    fn default() -> Self {
        Self::new()
    }
}

impl Styled for FrostedGlass {
    fn style(&mut self) -> &mut StyleRefinement {
        self.div.style()
    }
}

impl InteractiveElement for FrostedGlass {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.div.interactivity()
    }
}

impl ParentElement for FrostedGlass {
    fn extend(&mut self, elements: impl IntoIterator<Item = gpui::AnyElement>) {
        self.div.extend(elements);
    }
}

impl IntoElement for FrostedGlass {
    type Element = gpui::ViewElement<Self>;

    #[track_caller]
    fn into_element(self) -> Self::Element {
        gpui::ViewElement::new(self)
    }
}

fn shape_slot(shape: FrostedGlassShape, scale: f32) -> [f32; 4] {
    [
        shape.center.x.as_f32() * scale,
        shape.center.y.as_f32() * scale,
        shape.size.width.as_f32().max(0.0) * 0.5 * scale,
        shape.size.height.as_f32().max(0.0) * 0.5 * scale,
    ]
}

impl RenderOnce for FrostedGlass {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let appearance = self.appearance;
        let shapes = self.shapes;
        let shader = frosted_glass_shader();

        self.div.on_paint_before_children(
            move |bounds: Bounds<Pixels>, resolved_style, window: &mut Window, _| {
                let scale = window.raster_scale_factor();
                let corner_radii = resolved_style
                    .corner_radii
                    .to_pixels(window.rem_size())
                    .clamp_radii_for_quad_size(bounds.size);
                let automatic_radius = corner_radii
                    .top_left
                    .max(corner_radii.top_right)
                    .max(corner_radii.bottom_right)
                    .max(corner_radii.bottom_left);
                let automatic_shape = FrostedGlassShape::new(
                    Point {
                        x: bounds.size.width * 0.5,
                        y: bounds.size.height * 0.5,
                    },
                    bounds.size,
                    automatic_radius,
                );
                let first = shapes[0].unwrap_or(automatic_shape);
                let second = shapes[1].unwrap_or_default();
                let shape_count = if shapes[1].is_some() { 2.0 } else { 1.0 };
                let tint: Rgba = appearance.tint.into();
                let edge: Rgba = appearance.edge.into();
                let uniforms = EffectUniforms::new()
                    .with_slot(SHAPE_A_SLOT, shape_slot(first, scale))
                    .with_slot(SHAPE_B_SLOT, shape_slot(second, scale))
                    .with_slot(
                        SHAPE_CONFIG_SLOT,
                        [
                            first.radius.as_f32() * scale,
                            second.radius.as_f32() * scale,
                            appearance.merge_distance.as_f32() * scale,
                            shape_count,
                        ],
                    )
                    .with_slot(TINT_SLOT, [tint.r, tint.g, tint.b, tint.a])
                    .with_slot(
                        APPEARANCE_SLOT,
                        [
                            appearance.blur_radius.as_f32().max(0.0) * scale,
                            appearance.saturation.max(0.0),
                            appearance.brightness.max(0.0),
                            appearance.edge_width.as_f32().max(0.0) * scale,
                        ],
                    )
                    .with_slot(EDGE_SLOT, [edge.r, edge.g, edge.b, edge.a])
                    .with_slot(SURFACE_SLOT, [appearance.sheen.max(0.0), 0.0, 0.0, 0.0]);

                window.paint_backdrop_effect(
                    PaintBackdropEffect::new(bounds, px(0.0), shader.clone())
                        .uniforms(uniforms)
                        .corner_radii(Corners::default()),
                );
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{point, size};

    #[test]
    fn shape_slot_uses_half_extents_and_device_scale() {
        let shape = FrostedGlassShape::new(
            point(px(20.0), px(30.0)),
            size(px(80.0), px(40.0)),
            px(12.0),
        );
        assert_eq!(shape_slot(shape, 2.0), [40.0, 60.0, 80.0, 40.0]);
    }

    #[test]
    fn merge_records_two_shapes() {
        let shape = FrostedGlassShape::default();
        let glass = FrostedGlass::new().merge(shape, shape);
        assert!(glass.shapes.iter().all(Option::is_some));
    }

    #[test]
    fn presets_use_distinct_material_tints() {
        assert_ne!(
            FrostedGlassAppearance::dark().tint,
            FrostedGlassAppearance::light().tint
        );
    }

    #[test]
    fn appearance_fields_support_chainable_construction() {
        let appearance = FrostedGlassAppearance::dark()
            .blur_radius(px(12.0))
            .saturation(1.2)
            .merge_distance(px(52.0));

        assert_eq!(appearance.blur_radius, px(12.0));
        assert_eq!(appearance.saturation, 1.2);
        assert_eq!(appearance.merge_distance, px(52.0));
    }

    #[test]
    fn container_preserves_div_style_and_child_chaining() {
        let glass = FrostedGlass::new()
            .relative()
            .w(px(320.0))
            .px(px(12.0))
            .flex()
            .items_center()
            .text_color(hsla(0.0, 0.0, 1.0, 1.0))
            .opacity(0.8)
            .child(div());
        let _: gpui::ViewElement<FrostedGlass> = glass.into_element();
    }
}
