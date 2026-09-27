use std::time::{Duration, Instant};

use gpui::{Bounds, Pixels, Point, point, px, size};

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

#[derive(Clone, Copy)]
pub(super) struct PressScales {
    pub selection: f32,
    pub surface: f32,
}

impl Default for PressScales {
    fn default() -> Self {
        Self {
            selection: 1.35,
            surface: 1.05,
        }
    }
}

pub(super) fn scale_about_center(bounds: Bounds<Pixels>, scale: f32) -> Bounds<Pixels> {
    let extra = size(
        bounds.size.width * (scale - 1.),
        bounds.size.height * (scale - 1.),
    );
    Bounds::new(
        bounds.origin - point(extra.width / 2., extra.height / 2.),
        size(bounds.size.width * scale, bounds.size.height * scale),
    )
}

#[derive(Default)]
pub(super) struct Motion {
    pub press_scales: PressScales,
    axes: Option<[Spring; 4]>,
    target: Option<[f64; 4]>,
    last: Option<Instant>,
    press: Spring,
    press_target: f64,
    pub pressed: bool,
    press_until: Option<Instant>,
    pub pointer: Option<Point<Pixels>>,
    pub drag_bounds: Option<Bounds<Pixels>>,
}

pub(super) struct Visual {
    pub bounds: Bounds<Pixels>,
    pub pressure: f32,
    pub scale: f32,
    pub surface_scale: f32,
    pub moving: bool,
}

impl Motion {
    pub fn current_bounds(&self) -> Option<Bounds<Pixels>> {
        let axes = self.axes?;
        Some(Bounds::new(
            point(px(axes[0].value as f32), px(axes[1].value as f32)),
            size(px(axes[2].value as f32), px(axes[3].value as f32)),
        ))
    }

    pub fn begin_press(&mut self, now: Instant) {
        self.pressed = true;
        self.press_until = Some(now + Duration::from_millis(90));
    }

    pub fn cancel_press(&mut self) {
        self.pressed = false;
        self.press_until = None;
        self.drag_bounds = None;
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
        let Some(target) = self.drag_bounds.or(target) else {
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
            if !animated || self.drag_bounds.is_some() {
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
            moving |= self.press.value != pressure;
            moving |= holding;
        } else {
            self.press = Spring {
                value: pressure,
                velocity: 0.,
            };
        }
        self.press_target = pressure;
        let width = axes[2].value.max(1.);
        let height = axes[3].value.max(1.);
        let spread = self.press.value.clamp(-0.18, 1.12);
        let pressure = spread.clamp(0., 1.) as f32;
        let scale = 1. + spread as f32 * (self.press_scales.selection - 1.);
        let surface_scale = 1. + spread as f32 * (self.press_scales.surface - 1.);
        let bounds = scale_about_center(
            Bounds::new(
                point(px(axes[0].value as f32), px(axes[1].value as f32)),
                size(px(width as f32), px(height as f32)),
            ),
            scale,
        );
        Some(Visual {
            bounds,
            pressure,
            scale,
            surface_scale,
            moving,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn short_press_scales_uniformly_beyond_track_recovers_and_stops() {
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
            expanded |= sample.bounds.size.height > bounds.size.height + px(8.);
            rebounded |= sample.scale < 1.;
            let width_scale = sample.bounds.size.width / bounds.size.width;
            let height_scale = sample.bounds.size.height / bounds.size.height;
            assert!((width_scale - height_scale).abs() < 0.00001);
            assert!(f32::from(sample.bounds.center().x - bounds.center().x).abs() < 0.001);
            assert!(f32::from(sample.bounds.center().y - bounds.center().y).abs() < 0.001);
            visual = Some(sample);
        }
        assert!(expanded);
        assert!(rebounded);
        let visual = visual.unwrap();
        assert_eq!(visual.bounds, bounds);
        assert_eq!(visual.scale, 1.);
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
    fn press_scales_are_independent_centered_and_recover_together() {
        let start = Instant::now();
        let lens = Bounds::new(point(px(42.), px(8.)), size(px(100.), px(48.)));
        let track = Bounds::new(point(px(12.), px(20.)), size(px(440.), px(64.)));
        for (selection, surface) in [(1., 1.08), (1.6, 1.), (1.2, 1.1)] {
            let mut motion = Motion {
                press_scales: PressScales { selection, surface },
                ..Default::default()
            };
            motion.sample(Some(lens), start, true);
            motion.begin_press(start);
            for frame in 0..100 {
                let visual = motion
                    .sample(Some(lens), start + Duration::from_millis(frame * 16), true)
                    .unwrap();
                let outer = scale_about_center(track, visual.surface_scale);
                assert!(f32::from(outer.center().x - track.center().x).abs() < 0.001);
                assert!(f32::from(outer.center().y - track.center().y).abs() < 0.001);
                assert!(
                    (outer.size.width / track.size.width - outer.size.height / track.size.height)
                        .abs()
                        < 0.00001
                );
                if selection == 1. {
                    assert_eq!(visual.bounds, lens);
                }
                if surface == 1. {
                    assert_eq!(outer, track);
                }
            }
            let held = motion
                .sample(Some(lens), start + Duration::from_secs(2), true)
                .unwrap();
            assert!((held.scale - selection).abs() < 0.00001);
            assert!((held.surface_scale - surface).abs() < 0.00001);
            assert!(!held.moving);
            motion.cancel_press();
            for frame in 0..100 {
                motion.sample(
                    Some(lens),
                    start + Duration::from_millis(2000 + frame * 16),
                    true,
                );
            }
            let released = motion
                .sample(Some(lens), start + Duration::from_secs(4), true)
                .unwrap();
            assert_eq!(released.bounds, lens);
            assert_eq!(scale_about_center(track, released.surface_scale), track);
            assert!(!released.moving);
            motion.begin_press(start + Duration::from_secs(5));
            let immediate = motion
                .sample(Some(lens), start + Duration::from_secs(5), false)
                .unwrap();
            assert_eq!(immediate.scale, selection);
            assert_eq!(immediate.surface_scale, surface);
        }
    }

    #[test]
    fn drag_tracks_fractional_positions_then_springs_to_selection() {
        let mut motion = Motion::default();
        let start = Instant::now();
        let selected = Bounds::new(point(px(4.), px(4.)), size(px(100.), px(40.)));
        motion.sample(Some(selected), start, true);
        let dragged = Bounds::new(point(px(63.25), px(4.)), selected.size);
        motion.drag_bounds = Some(dragged);
        let now = start + Duration::from_millis(16);
        let visual = motion.sample(Some(selected), now, true).unwrap();
        assert_eq!(visual.bounds, dragged);
        assert!(!visual.moving);
        motion.drag_bounds = None;
        let target = Bounds::new(point(px(120.), px(4.)), size(px(150.), px(40.)));
        let released = motion.sample(Some(target), now, true).unwrap();
        assert_eq!(released.bounds, dragged);
        assert!(released.moving);
        for frame in 2..160 {
            motion.sample(
                Some(target),
                start + Duration::from_millis(frame * 16),
                true,
            );
        }
        assert_eq!(motion.current_bounds(), Some(target));
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
