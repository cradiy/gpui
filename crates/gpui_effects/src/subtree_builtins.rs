use gpui::{EffectShader, IntoElement, Pixels, px};

use crate::{EffectStage, SubtreeEffect, subtree_effect_chain};

/// Wave displacement in logical pixels, driven by `SubtreeEffect::time`.
#[derive(Clone, Copy, Debug)]
pub struct SubtreeWaveOptions {
    /// Maximum displacement along each axis.
    pub amplitude: Pixels,
    /// Distance between wave peaks.
    pub wavelength: Pixels,
    /// Temporal phase speed in radians per second. Negative values reverse motion.
    pub speed: f32,
}

impl Default for SubtreeWaveOptions {
    fn default() -> Self {
        Self {
            amplitude: px(6.),
            wavelength: px(180.),
            speed: 1.5,
        }
    }
}

/// Color controls applied without changing the captured alpha channel.
#[derive(Clone, Copy, Debug)]
pub struct SubtreeColorOptions {
    /// Zero is grayscale, one preserves saturation.
    pub saturation: f32,
    /// One preserves contrast; zero produces uniform mid-gray before brightness.
    pub contrast: f32,
    /// RGB multiplier. One preserves brightness.
    pub brightness: f32,
}

impl Default for SubtreeColorOptions {
    fn default() -> Self {
        Self {
            saturation: 1.,
            contrast: 1.,
            brightness: 1.,
        }
    }
}

/// Captures content with an identity image shader.
pub fn subtree_identity<E: IntoElement>(element: E) -> SubtreeEffect<E::Element> {
    subtree_effect_chain(element, [EffectStage::identity()])
}

/// Applies a compact 7 × 7 Gaussian blur with a logical-pixel support radius.
pub fn subtree_blur<E: IntoElement>(element: E, radius: Pixels) -> SubtreeEffect<E::Element> {
    subtree_effect_chain(element, [EffectStage::blur(radius)])
}

/// Applies continuous wave displacement with automatic capture padding.
pub fn subtree_wave<E: IntoElement>(
    element: E,
    options: SubtreeWaveOptions,
) -> SubtreeEffect<E::Element> {
    subtree_effect_chain(element, [EffectStage::wave(options)])
}

/// Adjusts saturation, contrast and brightness for the captured content.
pub fn subtree_color_adjust<E: IntoElement>(
    element: E,
    options: SubtreeColorOptions,
) -> SubtreeEffect<E::Element> {
    subtree_effect_chain(element, [EffectStage::color_adjust(options)])
}

impl EffectStage {
    /// Preserves the input pixels.
    pub fn identity() -> Self {
        Self::new(subtree_identity_shader()).pointer_transform(gpui::PointerTransform::identity())
    }

    /// Compact Gaussian blur. Radius is in logical pixels; zero preserves pixels.
    pub fn blur(radius: Pixels) -> Self {
        let radius = radius.max(px(0.));
        Self::new(subtree_blur_shader())
            .uniform_pixels(0, [radius, px(0.), px(0.), px(0.)])
            .pointer_transform(gpui::PointerTransform::identity())
            .capture_padding(radius)
    }

    /// Wave displacement driven by the chain's animation time.
    pub fn wave(options: SubtreeWaveOptions) -> Self {
        let amplitude = options.amplitude.max(px(0.));
        Self::new(subtree_wave_shader())
            .uniform_pixels(
                0,
                [amplitude, options.wavelength.max(px(1.)), px(0.), px(0.)],
            )
            .uniform(1, [options.speed, 0., 0., 0.])
            .capture_padding(amplitude + px(1.))
    }

    /// Saturation, contrast and brightness adjustment that preserves alpha.
    pub fn color_adjust(options: SubtreeColorOptions) -> Self {
        Self::new(subtree_color_adjust_shader())
            .uniform(
                0,
                [
                    options.saturation.max(0.),
                    options.contrast.max(0.),
                    options.brightness.max(0.),
                    0.,
                ],
            )
            .pointer_transform(gpui::PointerTransform::identity())
    }
}

/// Identity image shader; no uniforms are required.
pub fn subtree_identity_shader() -> EffectShader {
    EffectShader::wgsl_image(include_str!("shaders/subtree_identity.wgsl"))
}

/// Blur shader. Slot 0: `[radius_device_px, 0, 0, 0]`.
pub fn subtree_blur_shader() -> EffectShader {
    EffectShader::wgsl_image(include_str!("shaders/subtree_blur.wgsl"))
}

/// Wave shader. Slot 0: `[amplitude_device_px, wavelength_device_px, 0, 0]`;
/// slot 1: `[speed_radians_per_second, 0, 0, 0]`.
pub fn subtree_wave_shader() -> EffectShader {
    EffectShader::wgsl_image(include_str!("shaders/subtree_wave.wgsl"))
}

/// Color shader. Slot 0: `[saturation, contrast, brightness, 0]`.
pub fn subtree_color_adjust_shader() -> EffectShader {
    EffectShader::wgsl_image(include_str!("shaders/subtree_color_adjust.wgsl"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subtree_builtin_shaders_validate() {
        for shader in [
            subtree_identity_shader(),
            subtree_blur_shader(),
            subtree_wave_shader(),
            subtree_color_adjust_shader(),
            crate::bloom_extract_shader(),
            crate::bloom_blur_shader(),
            crate::bloom_composite_shader(),
            crate::feedback_shader(),
            crate::ripple_shader(),
            crate::lens_shader(),
            crate::deformation_shader(),
            crate::contour_glow_shader(),
            crate::contour_relief_shader(),
            crate::contour_shadow_shader(),
        ] {
            let source = gpui::compose_subtree_effect_wgsl(&shader);
            let module = naga::front::wgsl::parse_str(&source)
                .unwrap_or_else(|error| panic!("{}", error.emit_to_string(&source)));
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .expect("subtree shader must validate");
        }
        for shader in [
            crate::displacement_map_shader(),
            crate::masked_displacement_map_shader(),
        ] {
            let source = gpui::compose_subtree_image_effect_wgsl(&shader);
            let module = naga::front::wgsl::parse_str(&source)
                .unwrap_or_else(|error| panic!("{}", error.emit_to_string(&source)));
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .expect("external image stage must validate");
        }
    }
}
