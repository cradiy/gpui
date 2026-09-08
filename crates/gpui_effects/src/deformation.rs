use std::time::Duration;

use gpui::{Bounds, EffectShader, IntoElement, Pixels, Point, PointerTransform, point, px};

use crate::{EffectStage, SubtreeEffect, subtree_effect_chain};

/// A smooth local translation of a subtree's pixels.
#[derive(Clone, Copy, Debug)]
pub struct DeformationOptions {
    /// Anchor in normalized capture coordinates, including chain padding.
    pub center: Point<f32>,
    /// Distance over which deformation falls to zero, in logical pixels.
    pub radius: Pixels,
    /// Translation at the anchor. Length is limited to 35% of the radius
    /// to keep the mapping invertible and avoid folded content.
    pub offset: Point<Pixels>,
}

impl Default for DeformationOptions {
    fn default() -> Self {
        Self {
            center: point(0.5, 0.5),
            radius: px(260.),
            offset: point(px(0.), px(0.)),
        }
    }
}

fn finite_offset(offset: Point<Pixels>) -> Point<f32> {
    let offset = point(f32::from(offset.x), f32::from(offset.y));
    if offset.x.is_finite() && offset.y.is_finite() {
        offset
    } else {
        point(0., 0.)
    }
}

impl DeformationOptions {
    /// Maps a displayed point to its source position using the bounded deformation.
    pub fn source_position(
        &self,
        position: Point<Pixels>,
        bounds: Bounds<Pixels>,
    ) -> Point<Pixels> {
        let radius = f32::from(self.radius);
        let center = point(
            self.center.x * f32::from(bounds.size.width),
            self.center.y * f32::from(bounds.size.height),
        );
        let destination = (position - bounds.origin).map(f32::from);
        if !radius.is_finite()
            || radius <= 0.
            || !center.x.is_finite()
            || !center.y.is_finite()
            || (destination.x - center.x).hypot(destination.y - center.y) >= radius
        {
            return position;
        }
        let offset = self.constrained_offset().map(f32::from);
        let mut source = destination;
        for _ in 0..24 {
            let x = (source.x - center.x) / radius;
            let y = (source.y - center.y) / radius;
            let weight = (1. - x * x - y * y).max(0.).powi(3);
            source = destination - offset * weight;
        }
        bounds.origin + source.map(px)
    }

    /// Returns the bounded translation used by the renderer.
    pub fn constrained_offset(&self) -> Point<Pixels> {
        let offset = finite_offset(self.offset);
        let radius = f32::from(self.radius);
        if !radius.is_finite() || radius <= 0. {
            return point(px(0.), px(0.));
        }
        let length = f64::from(offset.x).hypot(f64::from(offset.y));
        let factor = (f64::from(radius) * 0.35 / length.max(f64::MIN_POSITIVE)).min(1.);
        point(
            px((f64::from(offset.x) * factor) as f32),
            px((f64::from(offset.y) * factor) as f32),
        )
    }
}

impl EffectStage {
    /// Deforms pixels without changing layout or accessibility. Pointer mapping is opt-in
    /// through [`SubtreeEffect::map_interaction`].
    /// The caller supplies pointer input and animation time. Invalid anchors or
    /// radii, and zero offsets, disable the stage.
    pub fn deformation(options: DeformationOptions) -> Self {
        let offset = options.constrained_offset();
        let radius = f32::from(options.radius);
        let enabled = options.center.x.is_finite()
            && options.center.y.is_finite()
            && radius.is_finite()
            && radius > 0.
            && offset != point(px(0.), px(0.));
        Self::new(deformation_shader())
            .uniform(0, [options.center.x, options.center.y, 0., 0.])
            .uniform_pixels(1, [offset.x, offset.y, options.radius, px(0.)])
            .pointer_transform(PointerTransform::new(move |position, bounds, _| {
                options.source_position(position, bounds)
            }))
            .enabled(enabled)
    }
}

/// Applies a local deformation to an element and all its painted descendants.
/// Leave transparent space around the content for its displaced edges.
pub fn subtree_deformation<E: IntoElement>(
    element: E,
    options: DeformationOptions,
) -> SubtreeEffect<E::Element> {
    subtree_effect_chain(element, [EffectStage::deformation(options)])
}

/// Slot 0: `[center_u, center_v, 0, 0]`;
/// slot 1: `[offset_x_device_px, offset_y_device_px, radius_device_px, 0]`.
pub fn deformation_shader() -> EffectShader {
    EffectShader::wgsl_image(include_str!("shaders/deformation.wgsl"))
}

/// Caller-clocked elastic translation. Dragging holds an offset; release returns
/// it to zero. This state does not own an element or schedule rendering.
#[derive(Clone, Debug)]
pub struct ElasticOffset {
    offset: Point<f32>,
    velocity: Point<f32>,
    held: bool,
    frequency: f32,
    damping: f32,
}

impl Default for ElasticOffset {
    fn default() -> Self {
        Self {
            offset: point(0., 0.),
            velocity: point(0., 0.),
            held: false,
            frequency: 2.8,
            damping: 0.64,
        }
    }
}

impl ElasticOffset {
    /// Sets natural frequency in hertz (0.1–20) and damping ratio (0.1–1).
    /// One gives a critically damped return without overshoot.
    pub fn spring(mut self, frequency: f32, damping: f32) -> Self {
        if frequency.is_finite() {
            self.frequency = frequency.clamp(0.1, 20.);
        }
        if damping.is_finite() {
            self.damping = damping.clamp(0.1, 1.);
        }
        self
    }

    /// Holds the current translation and stops its return velocity.
    pub fn grab(&mut self) {
        self.held = true;
        self.velocity = point(0., 0.);
    }

    /// Sets the held translation in logical pixels. Non-finite input is ignored.
    pub fn drag_to(&mut self, offset: Point<Pixels>) {
        if f32::from(offset.x).is_finite() && f32::from(offset.y).is_finite() {
            self.grab();
            self.offset = finite_offset(offset);
        }
    }

    /// Starts the return from the current translation.
    pub fn release(&mut self) {
        self.held = false;
    }

    /// Clears translation, velocity and the held state.
    pub fn clear(&mut self) {
        self.offset = point(0., 0.);
        self.velocity = point(0., 0.);
        self.held = false;
    }

    /// Current translation in logical pixels.
    pub fn offset(&self) -> Point<Pixels> {
        point(px(self.offset.x), px(self.offset.y))
    }

    /// Whether advancing the clock can change the translation.
    pub fn is_animating(&self) -> bool {
        !self.held && (self.offset != point(0., 0.) || self.velocity != point(0., 0.))
    }

    /// Advances the analytic spring and returns whether another frame is needed.
    pub fn advance(&mut self, elapsed: Duration) -> bool {
        if !self.is_animating() || elapsed.is_zero() {
            return self.is_animating();
        }
        let t = elapsed.as_secs_f64();
        let omega = f64::from(self.frequency) * std::f64::consts::TAU;
        let decay = omega * f64::from(self.damping);
        let envelope = (-decay * t).exp();
        let step = |position: f32, velocity: f32| {
            let x = f64::from(position);
            let v = f64::from(velocity);
            let (x, v) = if self.damping == 1. {
                let b = v + omega * x;
                ((x + b * t) * envelope, (v - omega * b * t) * envelope)
            } else {
                let frequency = omega * (1. - f64::from(self.damping).powi(2)).sqrt();
                let (sine, cosine) = (frequency * t).sin_cos();
                let b = (v + decay * x) / frequency;
                let y = x * cosine + b * sine;
                (
                    y * envelope,
                    (-x * frequency * sine + b * frequency * cosine - decay * y) * envelope,
                )
            };
            (x as f32, v as f32)
        };
        let (x, vx) = step(self.offset.x, self.velocity.x);
        let (y, vy) = step(self.offset.y, self.velocity.y);
        self.offset = point(x, y);
        self.velocity = point(vx, vy);
        if x.hypot(y) < 0.01 && vx.hypot(vy) < 0.05 {
            self.clear();
        }
        self.is_animating()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spring_is_time_partition_independent_and_settles() {
        for damping in [0.64, 1.] {
            let mut spring = ElasticOffset::default().spring(2.8, damping);
            spring.drag_to(point(px(80.), px(-40.)));
            spring.advance(Duration::from_secs(1));
            assert_eq!(spring.offset(), point(px(80.), px(-40.)));
            spring.release();
            let mut partitioned = spring.clone();
            spring.advance(Duration::from_millis(200));
            if damping < 1. {
                assert!(spring.offset.x < 0.);
            } else {
                assert!(spring.offset.x > 0.);
            }
            for _ in 0..20 {
                partitioned.advance(Duration::from_millis(10));
            }
            assert!((spring.offset.x - partitioned.offset.x).abs() < 0.001);
            assert!((spring.velocity.y - partitioned.velocity.y).abs() < 0.001);
            partitioned.grab();
            let held = partitioned.offset();
            partitioned.advance(Duration::from_secs(10));
            assert_eq!(held, partitioned.offset());
            spring.advance(Duration::from_secs(30));
            assert!(!spring.is_animating());
            assert_eq!(spring.offset(), point(px(0.), px(0.)));
        }
    }
}
