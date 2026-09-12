use gpui::{EffectShader, IntoElement, Pixels, SubtreeBloomPass, px};

use crate::{EffectStage, SubtreeEffect, subtree_effect_chain, subtree_identity_shader};

/// Highlight extraction and soft glow around captured content.
#[derive(Clone, Copy, Debug)]
pub struct BloomOptions {
    /// Highlight threshold, from zero to one.
    pub threshold: f32,
    /// Smooth transition around the threshold. Zero gives a hard cutoff.
    pub soft_knee: f32,
    /// Glow contribution. Zero disables the stage.
    pub intensity: f32,
    /// Gaussian support radius in logical pixels. Zero disables the stage.
    pub radius: Pixels,
    /// Texture size divisor: 1 for full resolution, 2 for half, 4 for quarter.
    /// Values are clamped to 1 through 8.
    pub downsample: u32,
}

impl Default for BloomOptions {
    fn default() -> Self {
        Self {
            threshold: 0.6,
            soft_knee: 0.15,
            intensity: 1.6,
            radius: px(56.),
            downsample: 4,
        }
    }
}

/// Adds colored highlight glow while retaining the original content.
pub fn subtree_bloom<E: IntoElement>(
    element: E,
    options: BloomOptions,
) -> SubtreeEffect<E::Element> {
    subtree_effect_chain(element, [EffectStage::bloom(options)])
}

impl EffectStage {
    /// Extracts highlights, blurs at reduced resolution and adds them to the input.
    pub fn bloom(options: BloomOptions) -> Self {
        let radius = options.radius.max(px(0.));
        let intensity = options.intensity.max(0.);
        let downsample = options.downsample.clamp(1, 8);
        let mut stage = Self::new(subtree_identity_shader())
            .uniform(
                0,
                [
                    options.threshold.clamp(0., 1.),
                    options.soft_knee.clamp(0., 1.),
                    intensity,
                    0.,
                ],
            )
            .uniform_pixels(1, [radius, px(0.), px(0.), px(0.)])
            .pointer_transform(gpui::PointerTransform::identity())
            .capture_padding(radius + px(downsample as f32))
            .enabled(radius > px(0.) && intensity > 0.);
        stage.bloom = Some(SubtreeBloomPass {
            extract: bloom_extract_shader(),
            blur: bloom_blur_shader(),
            composite: bloom_composite_shader(),
            downsample,
        });
        stage
    }
}

/// Highlight extraction. Slot 0: `[threshold, soft_knee, intensity, 0]`;
/// slot 2.zw: sampling footprint in source device pixels.
pub fn bloom_extract_shader() -> EffectShader {
    EffectShader::wgsl_image(include_str!("shaders/bloom_extract.wgsl"))
}

/// Separable Gaussian blur. Slot 1.x: support radius in device pixels;
/// slot 2.xy: blur axis.
pub fn bloom_blur_shader() -> EffectShader {
    EffectShader::wgsl_image(include_str!("shaders/bloom_blur.wgsl"))
}

/// Screen-blends blurred highlights from image two with image one. Slot 0.z: intensity.
pub fn bloom_composite_shader() -> EffectShader {
    EffectShader::wgsl_two_images(include_str!("shaders/bloom_composite.wgsl"))
}
