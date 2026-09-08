use gpui::{EffectShader, IntoElement, Pixels, px};

use crate::{EffectStage, MaterialLight, SubtreeEffect, subtree_effect_chain};

/// Beveled lighting derived from the alpha contour of painted content.
#[derive(Clone, Copy, Debug)]
pub struct ContourReliefOptions {
    /// Inward bevel width in logical pixels, clamped to 0 through 128.
    pub width: Pixels,
    /// Surface height in logical pixels, clamped to -128 through 128.
    /// Positive raises the shape; negative recesses it. Zero disables the effect.
    pub depth: Pixels,
    /// Specular highlight spread, clamped to 0 through 1.
    pub roughness: f32,
    /// Specular reflection strength, clamped to 0 through 2.
    pub specular: f32,
    /// Directional light in surface coordinates: right, down, toward the viewer.
    pub light: MaterialLight,
    /// Blend with the source, clamped to 0 through 1. Zero disables the effect.
    pub strength: f32,
    /// Alpha contour threshold, clamped to 0.001 through 0.999.
    pub threshold: f32,
}

impl Default for ContourReliefOptions {
    fn default() -> Self {
        Self {
            width: px(5.),
            depth: px(3.),
            roughness: 0.4,
            specular: 0.65,
            light: MaterialLight {
                direction: [-0.65, -0.6, 0.8],
                intensity: 1.2,
                ambient: 0.3,
                ..Default::default()
            },
            strength: 1.,
            threshold: 0.5,
        }
    }
}

fn bounded(value: f32, low: f32, high: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(low, high)
    } else {
        fallback
    }
}

impl EffectStage {
    /// Shades contour-derived bevels while preserving source alpha. Capture
    /// foreground content without an opaque background for per-glyph lighting.
    /// Layout and hit regions are unchanged; the caller drives light movement.
    pub fn contour_relief(options: ContourReliefOptions) -> Self {
        let width = bounded(f32::from(options.width), 0., 128., 0.);
        let depth = bounded(f32::from(options.depth), -128., 128., 0.);
        let strength = bounded(options.strength, 0., 1., 0.);
        let mut direction = options.light.direction;
        if !direction.iter().all(|v| v.is_finite()) || direction == [0.; 3] {
            direction = ContourReliefOptions::default().light.direction;
        }
        let max = direction.iter().fold(0_f32, |a, v| a.max(v.abs()));
        direction = direction.map(|v| v / max);
        let length = direction.iter().map(|v| v * v).sum::<f32>().sqrt();
        direction = direction.map(|v| v / length);
        let color = options.light.color;
        Self::distance_field(contour_relief_shader(), options.threshold)
            .uniform_pixels(0, [px(width), px(depth), px(0.75), px(0.)])
            .uniform(
                1,
                [
                    direction[0],
                    direction[1],
                    direction[2],
                    bounded(options.light.intensity, 0., 4., 1.),
                ],
            )
            .uniform(
                2,
                [
                    bounded(options.roughness, 0., 1., 0.4),
                    bounded(options.specular, 0., 2., 0.65),
                    strength,
                    bounded(options.light.ambient, 0., 1., 0.3),
                ],
            )
            .uniform(
                3,
                [
                    bounded(color.r, 0., 1., 1.),
                    bounded(color.g, 0., 1., 1.),
                    bounded(color.b, 0., 1., 1.),
                    0.,
                ],
            )
            .capture_padding(px(2.))
            .enabled(width > 0. && depth != 0. && strength > 0.)
    }
}

/// Applies raised or recessed contour lighting to an element subtree.
pub fn subtree_contour_relief<E: IntoElement>(
    element: E,
    options: ContourReliefOptions,
) -> SubtreeEffect<E::Element> {
    subtree_effect_chain(element, [EffectStage::contour_relief(options)])
}

/// WGSL functions `contour_height(distance, width, depth)` and
/// `contour_normal(input, width, depth, sample_step)` for distance-field stages.
/// All distances are device pixels. The normal points toward the viewer on a flat surface.
pub fn contour_surface_wgsl() -> &'static str {
    include_str!("shaders/contour_surface.wgsl")
}

/// Two-image relief shader. Slot 0: `[width_px, depth_px, sample_step_px, 0]`;
/// slot 1: `[light_x, light_y, light_z, intensity]`;
/// slot 2: `[roughness, specular, strength, ambient]`; slot 3.rgb: light color.
pub fn contour_relief_shader() -> EffectShader {
    EffectShader::wgsl_two_images(concat!(
        include_str!("shaders/contour_surface.wgsl"),
        "\n",
        include_str!("shaders/contour_relief.wgsl")
    ))
}
