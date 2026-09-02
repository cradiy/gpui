use std::time::{Duration, Instant};

use gpui::{Context, Pixels, Point, point, px};

use super::interaction::CaptureToken;

const RETURN_DURATION: Duration = Duration::from_millis(460);
const DISMISS_DURATION: Duration = Duration::from_millis(280);

#[derive(Clone, Copy, Debug)]
pub(super) struct BadgeVisual {
    pub(super) offset: Point<Pixels>,
    pub(super) opacity: f32,
    pub(super) active_motion: bool,
    pub(super) dismissing: bool,
    pub(super) completion: Option<(u64, bool)>,
}

#[derive(Clone, Copy, Debug)]
struct DragGesture {
    pointer_origin: Point<Pixels>,
    offset_origin: Point<Pixels>,
    last_pointer: Point<Pixels>,
    last_sample: Instant,
}

#[derive(Clone, Copy, Debug)]
struct Motion {
    id: u64,
    started_at: Instant,
    duration: Duration,
    from: Point<Pixels>,
    initial_velocity: Point<f32>,
    target: Point<Pixels>,
    dismissing: bool,
}

pub struct BadgeState {
    offset: Point<Pixels>,
    velocity: Point<f32>,
    drag: Option<DragGesture>,
    motion: Option<Motion>,
    next_motion_id: u64,
    dismissed: bool,
    capture: CaptureToken,
}

impl BadgeState {
    pub fn new() -> Self {
        Self {
            offset: Point::default(),
            velocity: Point::default(),
            drag: None,
            motion: None,
            next_motion_id: 1,
            dismissed: false,
            capture: CaptureToken::default(),
        }
    }

    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    pub fn is_dismissed(&self) -> bool {
        self.dismissed
    }

    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.offset = Point::default();
        self.velocity = Point::default();
        self.drag = None;
        self.motion = None;
        self.dismissed = false;
        cx.notify();
    }

    pub(super) fn capture(&self) -> CaptureToken {
        self.capture.clone()
    }

    pub(super) fn start_drag(&mut self, pointer: Point<Pixels>, cx: &mut Context<Self>) {
        if self.dismissed {
            return;
        }

        let now = Instant::now();
        let visual = self.visual(now);
        self.offset = visual.offset;
        self.velocity = Point::default();
        self.motion = None;
        self.drag = Some(DragGesture {
            pointer_origin: pointer,
            offset_origin: visual.offset,
            last_pointer: pointer,
            last_sample: now,
        });
        cx.notify();
    }

    pub(super) fn drag_to(&mut self, pointer: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(mut drag) = self.drag else {
            return;
        };
        let now = Instant::now();
        let elapsed = now
            .duration_since(drag.last_sample)
            .as_secs_f32()
            .max(0.001);
        let sample_velocity = point(
            f32::from(pointer.x - drag.last_pointer.x) / elapsed,
            f32::from(pointer.y - drag.last_pointer.y) / elapsed,
        );
        self.velocity = point(
            self.velocity.x * 0.68 + sample_velocity.x * 0.32,
            self.velocity.y * 0.68 + sample_velocity.y * 0.32,
        );
        self.offset = point(
            drag.offset_origin.x + pointer.x - drag.pointer_origin.x,
            drag.offset_origin.y + pointer.y - drag.pointer_origin.y,
        );
        drag.last_pointer = pointer;
        drag.last_sample = now;
        self.drag = Some(drag);
        cx.notify();
    }

    pub(super) fn end_drag(
        &mut self,
        pointer: Point<Pixels>,
        threshold: Pixels,
        cx: &mut Context<Self>,
    ) {
        if self.drag.is_none() {
            return;
        }
        self.drag_to(pointer, cx);
        self.drag = None;

        let projected = point(
            self.offset.x + px(self.velocity.x * 0.085),
            self.offset.y + px(self.velocity.y * 0.085),
        );
        let dismissing = length_pixels(projected) >= f32::from(threshold.max(px(1.)));
        let target = if dismissing {
            let direction = normalized(if length_pixels(projected) > 1.0 {
                projected
            } else {
                self.offset
            });
            let travel = 150.0 + (length_f32(self.velocity) * 0.055).min(170.0);
            point(
                self.offset.x + px(direction.x * travel),
                self.offset.y + px(direction.y * travel),
            )
        } else {
            Point::default()
        };
        let duration = if dismissing {
            DISMISS_DURATION
        } else {
            RETURN_DURATION
        };
        self.start_motion(target, duration, dismissing);
        cx.notify();
    }

    pub(super) fn visual(&self, now: Instant) -> BadgeVisual {
        if self.dismissed {
            return BadgeVisual {
                offset: self.offset,
                opacity: 0.0,
                active_motion: false,
                dismissing: false,
                completion: None,
            };
        }
        let Some(motion) = self.motion else {
            return BadgeVisual {
                offset: self.offset,
                opacity: drag_opacity(self.offset),
                active_motion: false,
                dismissing: false,
                completion: None,
            };
        };

        let phase = (now.duration_since(motion.started_at).as_secs_f32()
            / motion.duration.as_secs_f32())
        .clamp(0.0, 1.0);
        let (offset, opacity) = if motion.dismissing {
            let eased = 1.0 - (1.0 - phase).powi(4);
            (
                interpolate_point(motion.from, motion.target, eased),
                (1.0 - smoothstep((phase * 1.08).min(1.0))).max(0.0),
            )
        } else {
            (spring_position(motion, phase), 1.0)
        };
        BadgeVisual {
            offset,
            opacity,
            active_motion: phase < 1.0,
            dismissing: motion.dismissing,
            completion: (phase >= 1.0).then_some((motion.id, motion.dismissing)),
        }
    }

    pub(super) fn finish_motion(&mut self, id: u64, cx: &mut Context<Self>) -> bool {
        let Some(motion) = self.motion else {
            return false;
        };
        if motion.id != id {
            return false;
        }

        self.offset = motion.target;
        self.velocity = Point::default();
        self.motion = None;
        self.dismissed = motion.dismissing;
        cx.notify();
        motion.dismissing
    }

    fn start_motion(&mut self, target: Point<Pixels>, duration: Duration, dismissing: bool) {
        let id = self.next_motion_id;
        self.next_motion_id = self.next_motion_id.wrapping_add(1);
        self.motion = Some(Motion {
            id,
            started_at: Instant::now(),
            duration,
            from: self.offset,
            initial_velocity: self.velocity,
            target,
            dismissing,
        });
    }
}

impl Default for BadgeState {
    fn default() -> Self {
        Self::new()
    }
}

fn spring_position(motion: Motion, phase: f32) -> Point<Pixels> {
    let elapsed = phase * motion.duration.as_secs_f32();
    let decay = (-8.2 * elapsed).exp();
    let angular = 14.0;
    let spring_axis = |from: Pixels, target: Pixels, velocity: f32| {
        let displacement = f32::from(from - target);
        let oscillation = displacement * (angular * elapsed).cos()
            + (velocity / angular) * (angular * elapsed).sin();
        target + px(oscillation * decay)
    };
    point(
        spring_axis(motion.from.x, motion.target.x, motion.initial_velocity.x),
        spring_axis(motion.from.y, motion.target.y, motion.initial_velocity.y),
    )
}

fn drag_opacity(_offset: Point<Pixels>) -> f32 {
    1.0
}

fn interpolate_point(from: Point<Pixels>, to: Point<Pixels>, phase: f32) -> Point<Pixels> {
    point(
        from.x + (to.x - from.x) * phase,
        from.y + (to.y - from.y) * phase,
    )
}

fn length_pixels(value: Point<Pixels>) -> f32 {
    f32::from(value.x).hypot(f32::from(value.y))
}

fn length_f32(value: Point<f32>) -> f32 {
    value.x.hypot(value.y)
}

fn normalized(value: Point<Pixels>) -> Point<f32> {
    let length = length_pixels(value).max(0.001);
    point(f32::from(value.x) / length, f32::from(value.y) / length)
}

fn smoothstep(phase: f32) -> f32 {
    phase * phase * (3.0 - 2.0 * phase)
}
