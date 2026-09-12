use gpui::{IntoElement, Pixels, px};

use crate::{EffectStage, SubtreeEffect, subtree_effect_chain};

/// Blurs content near its top edge over 64 logical pixels, with a 12-pixel radius.
/// Style and size the content before wrapping it. No animation is scheduled.
pub fn progressive_blur<E: IntoElement>(content: E) -> ProgressiveBlur<E> {
    ProgressiveBlur {
        content,
        edge: 0,
        extent: px(64.),
        radius: px(12.),
        enabled: true,
    }
}

/// Spatially varying blur inside the content's capture bounds.
/// Layout, accessibility and pointer targets remain unchanged. Direction setters
/// select one edge; blur is strongest there and decreases smoothly inward.
pub struct ProgressiveBlur<E: IntoElement> {
    content: E,
    edge: u8,
    extent: Pixels,
    radius: Pixels,
    enabled: bool,
}

impl<E: IntoElement> ProgressiveBlur<E> {
    /// Blurs inward from the top over the supplied logical-pixel distance.
    pub fn top(mut self, extent: Pixels) -> Self {
        self.edge = 0;
        self.extent = extent;
        self
    }

    /// Blurs inward from the bottom over the supplied logical-pixel distance.
    pub fn bottom(mut self, extent: Pixels) -> Self {
        self.edge = 1;
        self.extent = extent;
        self
    }

    /// Blurs inward from the left over the supplied logical-pixel distance.
    pub fn left(mut self, extent: Pixels) -> Self {
        self.edge = 2;
        self.extent = extent;
        self
    }

    /// Blurs inward from the right over the supplied logical-pixel distance.
    pub fn right(mut self, extent: Pixels) -> Self {
        self.edge = 3;
        self.extent = extent;
        self
    }

    /// Maximum filter support radius, clamped to 0..=24 logical pixels.
    /// Zero, negative and non-finite radii disable the effect.
    pub fn radius(mut self, radius: Pixels) -> Self {
        self.radius = radius;
        self
    }

    /// Disabled effects paint their content directly without an offscreen pass.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Returns the subtree effect for composition with additional stages.
    pub fn into_effect(self) -> SubtreeEffect<E::Element> {
        subtree_effect_chain(
            self.content,
            stages(self.edge, self.extent, self.radius, self.enabled),
        )
    }
}

impl<E: IntoElement> IntoElement for ProgressiveBlur<E> {
    type Element = SubtreeEffect<E::Element>;

    fn into_element(self) -> Self::Element {
        self.into_effect()
    }
}

fn stages(edge: u8, extent: Pixels, radius: Pixels, enabled: bool) -> [EffectStage; 2] {
    let finite_positive = |value: Pixels| {
        let value = f32::from(value);
        if value.is_finite() { value.max(0.) } else { 0. }
    };
    let extent = finite_positive(extent);
    let radius = finite_positive(radius).min(24.);
    // Filter across the gradient first, then along it.
    let axes = if edge < 2 {
        [[1., 0.], [0., 1.]]
    } else {
        [[0., 1.], [1., 0.]]
    };
    axes.map(|axis| {
        EffectStage::new(progressive_blur_shader())
            .uniform_pixels(0, [px(extent), px(radius), px(0.), px(0.)])
            .uniform(1, [f32::from(edge), axis[0], axis[1], 0.])
            .pointer_transform(gpui::PointerTransform::identity())
            .enabled(enabled && extent > 0. && radius > 0.)
    })
}

/// One directional filter pass. Slot 0.xy: extent and radius in device pixels;
/// slot 1.xyz: edge (top, bottom, left, right) and sampling axis.
pub fn progressive_blur_shader() -> gpui::EffectShader {
    gpui::EffectShader::wgsl_image(include_str!("shaders/progressive_blur.wgsl"))
}

#[cfg(test)]
#[path = "progressive_blur_gpu_tests.rs"]
mod gpu_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_blur_configuration_bypasses_capture_and_scales_only_lengths() {
        for value in [f32::NAN, f32::INFINITY, -1., 0.] {
            assert!(
                stages(0, px(value), px(12.), true)
                    .iter()
                    .all(|s| !s.enabled)
            );
            assert!(
                stages(0, px(64.), px(value), true)
                    .iter()
                    .all(|s| !s.enabled)
            );
        }
        assert!(
            stages(0, px(64.), px(12.), false)
                .iter()
                .all(|s| !s.enabled)
        );
        for edge in 0..4 {
            let stages = stages(edge, px(80.), px(100.), true);
            for stage in stages {
                let prepared = stage.prepare(2., 0.);
                assert_eq!(prepared.uniforms.slots()[0], [160., 48., 0., 0.]);
                assert_eq!(prepared.uniforms.slots()[1], stage.uniforms.slots()[1]);
                assert!(stage.pointer_transform.is_some());
            }
        }
    }
}
