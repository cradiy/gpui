use gpui::{EffectShader, IntoElement, Pixels, Point, Rgba, point, px, rgba};

use crate::{EffectStage, SubtreeEffect, subtree_effect_chain};

/// A directional soft shadow projected from painted alpha contours.
#[derive(Clone, Copy, Debug)]
pub struct ContourShadowOptions {
    /// Projection vector in logical pixels, limited to a length of 256.
    pub offset: Point<Pixels>,
    /// Edge softness at the far end, clamped to 0 through 128 logical pixels.
    pub softness: Pixels,
    /// Edge softness at contact, clamped to 0 through 128 logical pixels.
    pub contact_softness: Pixels,
    /// Shadow color and opacity. The source is composited over the shadow.
    pub color: Rgba,
    /// Alpha contour threshold, clamped to 0.001 through 0.999.
    pub threshold: f32,
}

impl Default for ContourShadowOptions {
    fn default() -> Self {
        Self {
            offset: point(px(20.), px(28.)),
            softness: px(12.),
            contact_softness: px(1.),
            color: rgba(0x17223950),
            threshold: 0.5,
        }
    }
}

fn finite(value: f32) -> f32 {
    if value.is_finite() { value } else { 0. }
}

impl EffectStage {
    /// Projects a progressively softened contour shadow behind the source.
    /// Capture foreground content without an opaque panel background to shadow
    /// individual glyphs and icons. Layout and hit regions remain unchanged.
    pub fn contour_shadow(options: ContourShadowOptions) -> Self {
        let mut offset = [f32::from(options.offset.x), f32::from(options.offset.y)].map(finite);
        let largest = offset[0].abs().max(offset[1].abs());
        if largest > 256. {
            offset = offset.map(|value| value / largest * 256.);
        }
        let length = offset[0].hypot(offset[1]);
        if length > 256. {
            offset = offset.map(|value| value * (256. / length));
        }
        let softness = finite(f32::from(options.softness)).clamp(0., 128.);
        let contact = finite(f32::from(options.contact_softness)).clamp(0., 128.);
        let color = [
            options.color.r,
            options.color.g,
            options.color.b,
            options.color.a,
        ]
        .map(|value| finite(value).clamp(0., 1.));
        let padding = offset[0].abs().max(offset[1].abs()) + softness.max(contact) + 2.;
        Self::distance_field(contour_shadow_shader(), options.threshold)
            .uniform(0, color)
            .uniform_pixels(1, [px(offset[0]), px(offset[1]), px(contact), px(softness)])
            .capture_padding(px(padding))
            .enabled(color[3] > 0.)
    }
}

/// Adds a directional shadow following text, icon or image alpha contours.
pub fn subtree_contour_shadow<E: IntoElement>(
    element: E,
    options: ContourShadowOptions,
) -> SubtreeEffect<E::Element> {
    subtree_effect_chain(element, [EffectStage::contour_shadow(options)])
}

/// Two-image shadow shader. Slot 0: RGBA shadow color;
/// slot 1: `[offset_x_px, offset_y_px, contact_softness_px, softness_px]`.
pub fn contour_shadow_shader() -> EffectShader {
    EffectShader::wgsl_two_images(include_str!("shaders/contour_shadow.wgsl"))
}
