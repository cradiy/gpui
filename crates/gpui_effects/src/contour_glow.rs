use gpui::{EffectShader, IntoElement, Pixels, Rgba, SubtreeDistanceFieldPass, px, rgb};

use crate::{EffectStage, SubtreeEffect, subtree_effect_chain, subtree_identity_shader};

/// Light concentrated around an alpha contour, with a soft outer falloff.
#[derive(Clone, Copy, Debug)]
pub struct ContourGlowOptions {
    /// Glow color and opacity. Source colors are preserved.
    pub color: Rgba,
    /// Outer support radius in logical pixels, clamped to 0 through 256.
    pub radius: Pixels,
    /// Width of the bright edge in logical pixels, limited to the support radius.
    pub edge_width: Pixels,
    /// Light strength, clamped to 0 through 4. Zero disables the stage.
    pub intensity: f32,
    /// Alpha level defining the contour, clamped to 0.001 through 0.999.
    pub threshold: f32,
}

impl Default for ContourGlowOptions {
    fn default() -> Self {
        Self {
            color: rgb(0x76deff),
            radius: px(18.),
            edge_width: px(1.5),
            intensity: 1.3,
            threshold: 0.5,
        }
    }
}

impl EffectStage {
    /// Supplies the source image and its signed alpha-contour distance field to
    /// a two-image shader. Field R is distance in device pixels (negative inside),
    /// G indicates a valid contour, and alpha is one. Use `capture_padding` to
    /// reserve space for effects outside the source. No animation is scheduled.
    pub fn distance_field(composite: EffectShader, threshold: f32) -> Self {
        assert!(
            composite.image_count() == 2 && !composite.is_mask(),
            "distance fields require a two-image composite shader"
        );
        let mut stage = Self::new(subtree_identity_shader());
        stage.distance_field = Some(SubtreeDistanceFieldPass {
            composite,
            threshold,
        });
        stage
    }

    /// Lights the alpha contour of the input, including holes and disconnected
    /// shapes. Capture text or artwork without an opaque background to light
    /// their individual outlines. Layout and hit regions remain unchanged.
    pub fn contour_glow(options: ContourGlowOptions) -> Self {
        let radius = finite(f32::from(options.radius), 0.).clamp(0., 256.);
        let edge = finite(f32::from(options.edge_width), 0.).clamp(0., radius);
        let intensity = finite(options.intensity, 0.).clamp(0., 4.);
        let color = [
            options.color.r,
            options.color.g,
            options.color.b,
            options.color.a,
        ]
        .map(|value| finite(value, 0.).clamp(0., 1.));
        Self::distance_field(contour_glow_shader(), options.threshold)
            .uniform(0, color)
            .uniform_pixels(1, [px(radius), px(edge), px(0.), px(0.)])
            .uniform(2, [intensity, 0., 0., 0.])
            .capture_padding(px(radius + 2.))
            .enabled(radius > 0. && intensity > 0. && color[3] > 0.)
    }
}

fn finite(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

/// Adds contour light around an element's painted content.
pub fn subtree_contour_glow<E: IntoElement>(
    element: E,
    options: ContourGlowOptions,
) -> SubtreeEffect<E::Element> {
    subtree_effect_chain(element, [EffectStage::contour_glow(options)])
}

/// Two-image contour composite. Slot 0: RGBA light color; slot 1.xy: support
/// radius and edge width in device pixels; slot 2.x: light intensity.
pub fn contour_glow_shader() -> EffectShader {
    EffectShader::wgsl_two_images(include_str!("shaders/contour_glow.wgsl"))
}
