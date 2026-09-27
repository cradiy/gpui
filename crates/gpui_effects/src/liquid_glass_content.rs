use gpui::{
    App, Bounds, Corners, EffectShader, EffectUniforms, Hsla, IntoElement, Pixels, Rgba, Window, px,
};

use crate::{LiquidGlassDeformation, MaskedEffect, masked_effect};

/// A glass silhouette in logical window coordinates, shared with content effects.
#[derive(Clone, Copy, Debug)]
pub struct LiquidGlassRegion {
    /// Bounds before any contour deformation.
    pub bounds: Bounds<Pixels>,
    /// Corner radii, clamped to the undeformed bounds.
    pub corner_radii: Corners<Pixels>,
    /// The same deformation supplied to `paint_deformed_liquid_glass`.
    pub deformation: LiquidGlassDeformation,
}

impl LiquidGlassRegion {
    /// Creates an undeformed region. Assign `deformation` to follow a moving contour.
    pub fn new(bounds: Bounds<Pixels>, corner_radii: Corners<Pixels>) -> Self {
        Self {
            bounds,
            corner_radii,
            deformation: LiquidGlassDeformation::default(),
        }
    }

    fn uniforms(self, color: Hsla, window: &Window) -> EffectUniforms {
        let scale = window.scale_factor();
        let deformation = self.deformation.clamped(self.bounds.size);
        let center = self.bounds.center();
        let size = self.bounds.size;
        let corners = self
            .corner_radii
            .clamp_radii_for_quad_size(self.bounds.size);
        let color: Rgba = color.into();
        EffectUniforms::new()
            .with_slot(
                0,
                [
                    center.x.as_f32() * scale,
                    center.y.as_f32() * scale,
                    size.width.as_f32() * scale,
                    size.height.as_f32() * scale,
                ],
            )
            .with_slot(
                1,
                [
                    corners.top_left.as_f32() * scale,
                    corners.top_right.as_f32() * scale,
                    corners.bottom_right.as_f32() * scale,
                    corners.bottom_left.as_f32() * scale,
                ],
            )
            .with_slot(
                2,
                [
                    deformation.focus.x,
                    deformation.focus.y,
                    deformation.bulge.as_f32() * scale,
                    deformation.ripple.as_f32() * scale,
                ],
            )
            .with_slot(3, [color.r, color.g, color.b, color.a])
    }
}

/// Recolors covered portions of text and monochrome SVGs without changing layout.
///
/// The region is resolved at paint time and may be updated by an ancestor's paint
/// hook. `None` paints the content normally. Outside the region each glyph keeps
/// its original color. Inside, `color` supplies RGB and multiplies the original
/// alpha. Element opacity still applies once. Emoji and colored images retain
/// their normal rendering. This effect neither refracts nor captures its content.
pub fn liquid_glass_content<E: IntoElement>(
    content: E,
    color: Hsla,
    region: impl Fn(&mut Window, &mut App) -> Option<LiquidGlassRegion> + 'static,
) -> MaskedEffect<E::Element> {
    masked_effect(content, liquid_glass_content_shader()).uniforms_with(move |_, window, cx| {
        region(window, cx)
            .filter(|region| {
                region.bounds.size.width > px(0.) && region.bounds.size.height > px(0.)
            })
            .map(|region| region.uniforms(color, window))
    })
}

/// Mask shader for spatial glass-content coloring. Slots 0–3 contain region and
/// target color; slot 4 receives the original glyph or monochrome SVG color.
pub fn liquid_glass_content_shader() -> EffectShader {
    EffectShader::wgsl_mask(concat!(
        include_str!("shaders/liquid_glass_shape.wgsl"),
        "\n",
        include_str!("shaders/liquid_glass_content.wgsl"),
    ))
    .with_source_color_slot(4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_shader_validates_and_translates() {
        let source = gpui::compose_effect_shader_wgsl(&liquid_glass_content_shader());
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|error| panic!("{}", error.emit_to_string(&source)));
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("valid glass content shader");
        let (_, translation) = naga::back::msl::write_string(
            &module,
            &info,
            &naga::back::msl::Options {
                lang_version: (2, 0),
                ..Default::default()
            },
            &naga::back::msl::PipelineOptions::default(),
        )
        .expect("MSL translation");
        assert!(translation.entry_point_names.iter().all(Result::is_ok));
    }
}
