use gpui::{BorderGradient, Div, Pixels, Styled, div, px};

use crate::{BloomOptions, SubtreeEffect, subtree_bloom};

/// Colored light around a gradient border.
#[derive(Clone, Copy, Debug)]
pub struct BorderGlowOptions {
    /// Outer support radius in logical pixels.
    pub radius: Pixels,
    /// Light intensity. Zero draws only the original border.
    pub intensity: f32,
}

impl Default for BorderGlowOptions {
    fn default() -> Self {
        Self {
            radius: px(24.),
            intensity: 4.,
        }
    }
}

/// A transparent gradient border with matching colored glow.
///
/// Use `Styled` to set its size, border width and corner radii. Place it behind
/// an inset opaque surface to hide inward glow while keeping content sharp.
/// Ancestor clipping still applies to the outward glow.
pub fn border_glow(gradient: BorderGradient, options: BorderGlowOptions) -> SubtreeEffect<Div> {
    subtree_bloom(
        div().border_1().border_gradient(gradient),
        BloomOptions {
            threshold: 0.,
            soft_knee: 0.,
            intensity: options.intensity,
            radius: options.radius,
            downsample: 2,
        },
    )
}
