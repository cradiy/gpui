use std::time::{Duration, Instant};

use gpui::{Bounds, Pixels, Point, point, px, size};
use gpui_effects::LiquidGlassDeformation;

#[derive(Clone, Copy, Default)]
struct Spring {
    value: f64,
    velocity: f64,
}

impl Spring {
    fn step(&mut self, target: f64, dt: f64, frequency: f64, damping: f64) -> bool {
        let offset = self.value - target;
        let decay = (-frequency * damping * dt).exp();
        let damped = frequency * (1.0 - damping * damping).sqrt();
        let phase = damped * dt;
        let sine = phase.sin() / damped;
        let velocity = self.velocity;
        self.value = target
            + decay * (offset * phase.cos() + (velocity + frequency * damping * offset) * sine);
        self.velocity = decay
            * (velocity * phase.cos()
                - (frequency * damping * velocity + frequency * frequency * offset) * sine);
        if (self.value - target).abs() < 0.01 && self.velocity.abs() < 0.02 {
            self.value = target;
            self.velocity = 0.;
            false
        } else {
            true
        }
    }
}

#[derive(Default)]
pub(super) struct Motion {
    axes: Option<[Spring; 4]>,
    target: Option<[f64; 4]>,
    last: Option<Instant>,
    press: Spring,
    vertical_press: Spring,
    press_target: f64,
    pub pressed: bool,
    press_until: Option<Instant>,
    pub press_focus: Point<f32>,
    pub pointer: Option<Point<Pixels>>,
}

pub(super) struct Visual {
    pub bounds: Bounds<Pixels>,
    pub pressure: f32,
    pub deformation: LiquidGlassDeformation,
    pub moving: bool,
}

impl Motion {
    pub fn begin_press(&mut self, now: Instant) {
        self.pressed = true;
        self.press_until = Some(now + Duration::from_millis(90));
    }

    pub fn cancel_press(&mut self) {
        self.pressed = false;
        self.press_until = None;
    }

    pub fn sample(
        &mut self,
        target: Option<Bounds<Pixels>>,
        now: Instant,
        animated: bool,
    ) -> Option<Visual> {
        let dt = self
            .last
            .map_or(0., |last| now.saturating_duration_since(last).as_secs_f64());
        self.last = Some(now);
        let Some(target) = target else {
            self.axes = None;
            self.target = None;
            return None;
        };
        let values = [
            target.origin.x,
            target.origin.y,
            target.size.width,
            target.size.height,
        ]
        .map(|v| f64::from(f32::from(v)));
        let axes = self.axes.get_or_insert(values.map(|value| Spring {
            value,
            velocity: 0.,
        }));
        let previous = self.target.unwrap_or(values);
        self.target = Some(values);
        let mut moving = false;
        for ((axis, target), previous) in axes.iter_mut().zip(values).zip(previous) {
            if !animated {
                *axis = Spring {
                    value: target,
                    velocity: 0.,
                };
            } else {
                moving |= axis.step(previous, dt, 24., 0.82);
                moving |= axis.value != target;
            }
        }
        let holding = animated && self.press_until.is_some_and(|until| now < until);
        let pressure = if self.pressed || holding { 1. } else { 0. };
        if animated {
            let (frequency, damping) = if self.press_target > 0. {
                (36., 0.78)
            } else {
                (26., 0.72)
            };
            moving |= self.press.step(self.press_target, dt, frequency, damping);
            moving |= self.vertical_press.step(self.press_target, dt, 28., 0.78);
            moving |= self.press.value != pressure;
            moving |= self.vertical_press.value != pressure;
            moving |= holding;
        } else {
            self.press = Spring {
                value: pressure,
                velocity: 0.,
            };
            self.vertical_press = self.press;
        }
        self.press_target = pressure;
        let width = axes[2].value.max(1.);
        let stretch = (axes[0].velocity.abs() * 0.008).min(width * 0.09);
        let spread = self.press.value.clamp(-0.18, 1.12);
        let pressure = spread.clamp(0., 1.) as f32;
        let bulge = spread * (axes[3].value * 0.12).min(5.);
        let ripple = (self.press.value - self.vertical_press.value).clamp(-0.6, 0.6)
            * (axes[3].value * 0.05).min(2.);
        let bounds = Bounds::new(
            point(
                px((axes[0].value - stretch * 0.5) as f32),
                px(axes[1].value as f32),
            ),
            size(
                px((width + stretch).max(1.) as f32),
                px(axes[3].value.max(1.) as f32),
            ),
        );
        Some(Visual {
            bounds,
            pressure,
            deformation: LiquidGlassDeformation {
                focus: self.press_focus,
                bulge: px(bulge as f32),
                ripple: px(ripple as f32),
            },
            moving,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn short_press_spreads_recovers_and_stops() {
        let mut motion = Motion::default();
        let start = Instant::now();
        let bounds = Bounds::new(point(px(4.), px(4.)), size(px(100.), px(40.)));
        motion.sample(Some(bounds), start, true);
        motion.begin_press(start);
        motion.sample(Some(bounds), start, true);
        motion.pressed = false;
        let mut expanded = false;
        let mut rebounded = false;
        let mut visual = None;
        for frame in 1..150 {
            let sample = motion
                .sample(Some(bounds), start + Duration::from_millis(frame * 8), true)
                .unwrap();
            expanded |= sample.deformation.bulge > px(3.);
            rebounded |= sample.deformation.bulge < px(0.);
            assert_eq!(sample.bounds, bounds);
            assert!(f32::from(sample.bounds.center().x - bounds.center().x).abs() < 0.001);
            assert!(f32::from(sample.bounds.center().y - bounds.center().y).abs() < 0.001);
            visual = Some(sample);
        }
        assert!(expanded);
        assert!(rebounded);
        let visual = visual.unwrap();
        assert_eq!(visual.bounds, bounds);
        assert_eq!(visual.deformation.bulge, px(0.));
        assert_eq!(visual.deformation.ripple, px(0.));
        assert!(!visual.moving);

        motion.begin_press(start + Duration::from_secs(2));
        motion.cancel_press();
        let visual = motion
            .sample(Some(bounds), start + Duration::from_secs(2), true)
            .unwrap();
        assert_eq!(visual.bounds, bounds);
        assert!(!visual.moving);

        motion.begin_press(start + Duration::from_secs(3));
        motion.sample(Some(bounds), start + Duration::from_secs(3), false);
        motion.pressed = false;
        let visual = motion
            .sample(Some(bounds), start + Duration::from_secs(3), false)
            .unwrap();
        assert_eq!(visual.bounds, bounds);
        assert!(!visual.moving);
    }

    #[test]
    fn interrupted_motion_keeps_position_and_velocity_then_settles() {
        let mut motion = Motion::default();
        let start = Instant::now();
        let a = Bounds::new(point(px(4.), px(4.)), size(px(70.), px(36.)));
        let b = Bounds::new(point(px(150.), px(4.)), size(px(130.), px(36.)));
        motion.sample(Some(a), start, true);
        motion.sample(Some(b), start + Duration::from_millis(40), true);
        motion.sample(Some(b), start + Duration::from_millis(80), true);
        let before = motion.axes.unwrap();
        assert!(before[0].velocity > 0.);
        motion.sample(Some(a), start + Duration::from_millis(80), true);
        for (a, b) in before.iter().zip(motion.axes.unwrap()) {
            assert_eq!(a.value, b.value);
            assert_eq!(a.velocity, b.velocity);
        }
        let mut visual = None;
        for frame in 6..160 {
            visual = motion.sample(Some(a), start + Duration::from_millis(frame * 16), true);
        }
        let visual = visual.unwrap();
        assert!(!visual.moving);
        assert_eq!(visual.bounds, a);
        let wake = motion
            .sample(Some(b), start + Duration::from_secs(3), true)
            .unwrap();
        assert_eq!(wake.bounds, a);
        assert!(wake.moving);
        assert_eq!(
            motion
                .sample(Some(b), start + Duration::from_secs(3), false)
                .unwrap()
                .bounds,
            b
        );
        assert!(
            motion
                .sample(None, start + Duration::from_secs(4), true)
                .is_none()
        );
        assert_eq!(
            motion
                .sample(Some(a), start + Duration::from_secs(4), true)
                .unwrap()
                .bounds,
            a
        );
    }
}
