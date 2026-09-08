use gpui::{Bounds, EffectShader, IntoElement, Pixels, Point, PointerTransform, point, px};

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
    /// are supplied by the caller. Invalid centers or radii and unit scale disable it.
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
            .pointer_transform(PointerTransform::new(move |position, bounds, scale| {
                options.source_position(position, bounds, scale)
            }))
            .enabled(
                options.center.x.is_finite()
                    && options.center.y.is_finite()
                    && f32::from(radius).is_finite()
                    && radius > px(0.)
                    && magnification != 1.,
            )
    }
}

impl LensOptions {
    /// Maps a displayed point to the sampled source position using the lens shader's geometry.
    pub fn source_position(
        &self,
        position: Point<Pixels>,
        bounds: Bounds<Pixels>,
        scale_factor: f32,
    ) -> Point<Pixels> {
        let size = bounds.size.map(f32::from);
        let local = (position - bounds.origin).map(f32::from);
        let radius = f32::from(self.radius);
        let zoom = if self.magnification.is_finite() {
            self.magnification.clamp(0.5, 3.)
        } else {
            1.
        };
        let dx = local.x - self.center.x * size.width;
        let dy = local.y - self.center.y * size.height;
        let distance = dx.hypot(dy);
        if size.width <= 0.
            || size.height <= 0.
            || radius <= 0.
            || !radius.is_finite()
            || !distance.is_finite()
            || distance >= radius
            || zoom == 1.
        {
            return position;
        }
        let softness = if self.softness.is_finite() {
            self.softness.clamp(0., 1.)
        } else {
            0.5
        };
        let weight = (1. - (distance / radius).powi(2))
            .max(0.)
            .powf(3. + softness * 3.);
        let edge_distance = local
            .x
            .min(local.y)
            .min(size.width - local.x)
            .min(size.height - local.y);
        let edge_fade = f32::from(self.edge_fade.max(px(1.))).max(1. / scale_factor.max(0.001));
        let t = (edge_distance / edge_fade).clamp(0., 1.);
        let scale = (-zoom.ln() * weight * t * t * (3. - 2. * t)).exp();
        let inset = 0.5 / scale_factor.max(0.001);
        bounds.origin
            + point(
                px((local.x + dx * (scale - 1.)).clamp(
                    inset.min(size.width / 2.),
                    (size.width - inset).max(size.width / 2.),
                )),
                px((local.y + dy * (scale - 1.)).clamp(
                    inset.min(size.height / 2.),
                    (size.height - inset).max(size.height / 2.),
                )),
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
