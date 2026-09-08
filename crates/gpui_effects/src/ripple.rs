use std::time::Duration;

use gpui::{EffectShader, IntoElement, Pixels, Point, px};

use crate::{EffectStage, SubtreeEffect, subtree_effect_chain};

/// Maximum number of overlapping waves in one ripple stage.
pub const MAX_RIPPLES: usize = 4;

/// One outward-propagating wave, driven by an application clock.
#[derive(Clone, Copy, Debug)]
pub struct Ripple {
    /// Normalized center within the capture bounds, including any chain padding.
    pub center: Point<f32>,
    /// Time since this wave was emitted.
    pub elapsed: Duration,
}

/// Spatial and temporal controls for radial displacement.
#[derive(Clone, Copy, Debug)]
pub struct RippleOptions {
    /// Maximum combined displacement in logical pixels.
    pub amplitude: Pixels,
    /// Distance between wave peaks in logical pixels.
    pub wavelength: Pixels,
    /// Half-width of the moving wave packet in logical pixels.
    pub width: Pixels,
    /// Outward travel distance per second in logical pixels.
    pub speed: Pixels,
    /// Lifetime of each wave, including its fade-out.
    pub duration: Duration,
    /// Distance over which displacement fades near the capture edges.
    pub edge_fade: Pixels,
}

impl Default for RippleOptions {
    fn default() -> Self {
        Self {
            amplitude: px(12.),
            wavelength: px(72.),
            width: px(110.),
            speed: px(340.),
            duration: Duration::from_millis(2200),
            edge_fade: px(32.),
        }
    }
}

impl EffectStage {
    /// Combines radial waves in one sampling pass. The last four live waves are used.
    /// With no live waves or zero amplitude, input pixels are preserved.
    pub fn ripples(options: RippleOptions, ripples: impl IntoIterator<Item = Ripple>) -> Self {
        let duration = options.duration.max(Duration::from_millis(1));
        let mut waves = [[0.; 4]; MAX_RIPPLES];
        let mut count = 0;
        for ripple in ripples {
            if ripple.elapsed >= duration
                || !ripple.center.x.is_finite()
                || !ripple.center.y.is_finite()
            {
                continue;
            }
            if count == MAX_RIPPLES {
                waves.rotate_left(1);
                count -= 1;
            }
            waves[count] = [
                ripple.center.x,
                ripple.center.y,
                ripple.elapsed.as_secs_f32(),
                1.,
            ];
            count += 1;
        }
        let amplitude = options.amplitude.max(px(0.));
        let mut stage = Self::new(ripple_shader())
            .uniform_pixels(
                0,
                [
                    amplitude,
                    options.wavelength.max(px(1.)),
                    options.width.max(px(1.)),
                    options.speed.max(px(0.)),
                ],
            )
            .uniform(1, [duration.as_secs_f32(), 0., 0., 0.])
            .uniform_pixels(6, [options.edge_fade.max(px(1.)), px(0.), px(0.), px(0.)])
            .enabled(count > 0 && amplitude > px(0.));
        for (index, wave) in waves.into_iter().enumerate() {
            stage = stage.uniform(index + 2, wave);
        }
        stage
    }
}

/// Applies radial displacement to a captured element subtree.
pub fn subtree_ripples<E: IntoElement>(
    element: E,
    options: RippleOptions,
    ripples: impl IntoIterator<Item = Ripple>,
) -> SubtreeEffect<E::Element> {
    subtree_effect_chain(element, [EffectStage::ripples(options, ripples)])
}

/// Radial displacement shader. Slot 0 contains device-pixel amplitude, wavelength,
/// packet half-width and speed; slot 1 contains lifetime in seconds. Slots 2–5
/// contain `[center_u, center_v, elapsed_seconds, enabled]`; slot 6 contains edge fade.
pub fn ripple_shader() -> EffectShader {
    EffectShader::wgsl_image(include_str!("shaders/ripple.wgsl"))
}
