use std::time::Duration;

use gpui::{EffectShader, IntoElement, Pixels, Point, PointerTransform, point, px};

use crate::{EffectStage, SubtreeEffect, subtree_effect_chain};

/// Centered directional blur for a translating subtree.
#[derive(Clone, Copy, Debug)]
pub struct MotionBlurOptions {
    /// Translation velocity in logical pixels per second, in capture axes.
    pub velocity: Point<Pixels>,
    /// Shutter duration, independent of the frame interval.
    pub exposure: Duration,
    /// Exposure multiplier, clamped to 0 through 8. Zero disables the stage.
    pub strength: f32,
    /// Total blur support length in logical pixels, clamped to 0 through 128.
    pub max_distance: Pixels,
    /// Sampling budget, clamped to 3 through 129 and rounded up to an odd count.
    pub samples: u32,
}

impl Default for MotionBlurOptions {
    fn default() -> Self {
        Self {
            velocity: point(px(0.), px(0.)),
            exposure: Duration::from_secs_f64(1. / 120.),
            strength: 1.,
            max_distance: px(24.),
            samples: 33,
        }
    }
}

/// Applies a velocity-driven blur without moving layout or pointer targets.
pub fn subtree_motion_blur<E: IntoElement>(
    element: E,
    options: MotionBlurOptions,
) -> SubtreeEffect<E::Element> {
    subtree_effect_chain(element, [EffectStage::motion_blur(options)])
}

impl EffectStage {
    /// Samples symmetrically along the translation vector with a soft shutter profile.
    pub fn motion_blur(options: MotionBlurOptions) -> Self {
        let bounded = |value: f32, limit: f32| {
            if value.is_finite() {
                value.clamp(0., limit)
            } else {
                0.
            }
        };
        let x = f64::from(f32::from(options.velocity.x));
        let y = f64::from(f32::from(options.velocity.y));
        let speed = x.hypot(y);
        let distance =
            (speed * options.exposure.as_secs_f64() * f64::from(bounded(options.strength, 8.)))
                .min(f64::from(bounded(f32::from(options.max_distance), 128.)));
        let (dx, dy) = if speed.is_finite() && speed > 0. && distance > 0. {
            ((x / speed * distance) as f32, (y / speed * distance) as f32)
        } else {
            (0., 0.)
        };
        Self::new(motion_blur_shader())
            .uniform_pixels(0, [px(dx), px(dy), px(0.), px(0.)])
            .uniform(1, [(options.samples.clamp(3, 129) | 1) as f32, 0., 0., 0.])
            .pointer_transform(PointerTransform::identity())
            .capture_padding(px(dx.abs().max(dy.abs()) * 0.5 + 1.))
            .enabled(dx != 0. || dy != 0.)
    }
}

/// Slot 0.xy: full shutter displacement in device pixels; slot 1.x: sample budget.
pub fn motion_blur_shader() -> EffectShader {
    EffectShader::wgsl_image(include_str!("shaders/motion_blur.wgsl"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shutter_preserves_direction_and_scales_only_pixel_parameters() {
        let stage = EffectStage::motion_blur(MotionBlurOptions {
            velocity: point(px(-3000.), px(4000.)),
            exposure: Duration::from_millis(100),
            max_distance: px(20.),
            samples: 32,
            ..Default::default()
        });
        for scale in [1., 1.5, 2.] {
            let prepared = stage.prepare(scale, 0.);
            let slots = prepared.uniforms.slots();
            assert_eq!(slots[0], [-12. * scale, 16. * scale, 0., 0.]);
            assert_eq!(slots[1][0], 33.);
            assert!(f32::from(stage.padding) * scale >= slots[0][0].abs() * 0.5);
            assert!(f32::from(stage.padding) * scale >= slots[0][1].abs() * 0.5);
        }
    }

    #[test]
    fn inactive_and_invalid_motion_skip_capture() {
        let moving = MotionBlurOptions {
            velocity: point(px(900.), px(-200.)),
            ..Default::default()
        };
        for options in [
            MotionBlurOptions::default(),
            MotionBlurOptions {
                exposure: Duration::ZERO,
                ..moving
            },
            MotionBlurOptions {
                strength: 0.,
                ..moving
            },
            MotionBlurOptions {
                strength: f32::NAN,
                ..moving
            },
            MotionBlurOptions {
                max_distance: px(-1.),
                ..moving
            },
            MotionBlurOptions {
                velocity: point(px(f32::INFINITY), px(0.)),
                ..moving
            },
        ] {
            let stage = EffectStage::motion_blur(options);
            assert!(!stage.enabled);
            assert!(
                stage
                    .prepare(2., 0.)
                    .uniforms
                    .slots()
                    .iter()
                    .flatten()
                    .all(|v| v.is_finite())
            );
        }
    }
}
