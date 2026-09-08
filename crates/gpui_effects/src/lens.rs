use gpui::{EffectShader, IntoElement, Pixels, Point, point, px};

use crate::{EffectStage, SubtreeEffect, subtree_effect_chain};

/// Local magnification with a smooth transition to unchanged surrounding pixels.
#[derive(Clone, Copy, Debug)]
pub struct LensOptions {
    /// Normalized center within the capture bounds, including chain padding.
    pub center: Point<f32>,
    /// Support radius in logical pixels. Pixels outside this radius are unchanged.
    pub radius: Pixels,
    /// Center scale, clamped to 0.5 through 3.0. One preserves the input;
    /// values above one magnify, and values below one compress.
    pub magnification: f32,
    /// Falloff softness, clamped to 0 through 1. Higher values concentrate the
    /// full-strength region near the center and leave a gentler outer transition.
    pub softness: f32,
    /// Distance over which displacement fades near capture edges, in logical pixels.
    pub edge_fade: Pixels,
}

impl Default for LensOptions {
    fn default() -> Self {
        Self {
            center: point(0.5, 0.5),
            radius: px(220.),
            magnification: 1.8,
            softness: 0.5,
            edge_fade: px(32.),
        }
    }
}

impl EffectStage {
    /// Applies a borderless lens to the stage input. Animation and pointer tracking
    /// are supplied by the caller. Invalid centers, zero radius and unit scale disable it.
    pub fn lens(options: LensOptions) -> Self {
        let magnification = if options.magnification.is_finite() {
            options.magnification.clamp(0.5, 3.)
        } else {
            1.
        };
        let softness = if options.softness.is_finite() {
            options.softness.clamp(0., 1.)
        } else {
            0.5
        };
        let radius = options.radius.max(px(0.));
        Self::new(lens_shader())
            .uniform(
                0,
                [options.center.x, options.center.y, magnification, softness],
            )
            .uniform_pixels(1, [radius, options.edge_fade.max(px(1.)), px(0.), px(0.)])
            .enabled(
                options.center.x.is_finite()
                    && options.center.y.is_finite()
                    && radius > px(0.)
                    && magnification != 1.,
            )
    }
}

/// Magnifies or compresses a local region of an element subtree.
pub fn subtree_lens<E: IntoElement>(element: E, options: LensOptions) -> SubtreeEffect<E::Element> {
    subtree_effect_chain(element, [EffectStage::lens(options)])
}

/// Lens shader. Slot 0: `[center_u, center_v, magnification, softness]`;
/// slot 1: `[radius_device_px, edge_fade_device_px, 0, 0]`.
pub fn lens_shader() -> EffectShader {
    EffectShader::wgsl_image(include_str!("shaders/lens.wgsl"))
}
