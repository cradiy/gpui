use gpui::{EffectShader, EffectUniforms, ImageSource, Point, point};

use crate::{Effect, two_image_effect};

/// View displacement and depth sampling for a paired color image and depth map.
#[derive(Clone, Copy, Debug)]
pub struct DepthParallaxOptions {
    /// View offset in -1 through 1 on each axis. Positive X moves near content right.
    pub offset: Point<f32>,
    /// Full depth-range displacement as a fraction of each viewport dimension, limited to 0–0.15.
    pub strength: f32,
    /// Stationary depth plane in 0–1, after depth inversion.
    pub focus: f32,
    /// Interpret black as near and white as far.
    pub invert_depth: bool,
    /// Front-to-back depth search steps, limited to 8–64.
    pub steps: u32,
}

impl Default for DepthParallaxOptions {
    fn default() -> Self {
        Self {
            offset: point(0., 0.),
            strength: 0.045,
            focus: 0.5,
            invert_depth: false,
            steps: 32,
        }
    }
}

impl DepthParallaxOptions {
    /// Packs view parameters without device-scale conversion.
    pub fn uniforms(self) -> EffectUniforms {
        let finite = |value: f32, min: f32, max: f32, fallback: f32| {
            if value.is_finite() {
                value.clamp(min, max)
            } else {
                fallback
            }
        };
        EffectUniforms::new()
            .with_slot(
                0,
                [
                    finite(self.offset.x, -1., 1., 0.),
                    finite(self.offset.y, -1., 1., 0.),
                    finite(self.strength, 0., 0.15, 0.),
                    finite(self.focus, 0., 1., 0.5),
                ],
            )
            .with_slot(
                1,
                [
                    f32::from(self.invert_depth),
                    self.steps.clamp(8, 64) as f32,
                    0.,
                    0.,
                ],
            )
    }
}

/// Displays a cover-fitted image with depth-driven view displacement.
/// The depth map must be registered to the image; white is near by default.
pub fn depth_parallax(
    image: impl Into<ImageSource>,
    depth: impl Into<ImageSource>,
    options: DepthParallaxOptions,
) -> Effect {
    two_image_effect(image, depth, depth_parallax_shader()).uniforms(options.uniforms())
}

/// Two-image shader: color followed by linear depth data in the red channel.
/// Slot 0: `[offset_x, offset_y, strength, focus]`; slot 1: `[invert_depth, steps, 0, 0]`.
pub fn depth_parallax_shader() -> EffectShader {
    EffectShader::wgsl_two_images(include_str!("shaders/depth_parallax.wgsl"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depth_shader_validates() {
        let source = gpui::compose_effect_shader_wgsl(&depth_parallax_shader());
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|error| panic!("{}", error.emit_to_string(&source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("depth parallax shader must validate");
    }
}
