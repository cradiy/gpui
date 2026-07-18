use gpui::prelude::*;
use gpui::{
    AnyElement, Context, EventEmitter, IntoElement, Pixels, Point, Render, Transformation, Window,
    div, point,
};
use std::{
    collections::VecDeque,
    rc::Rc,
    time::{Duration, Instant},
};

/// Identifies one group of coordinated motion items.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MotionId(u64);

impl MotionId {
    /// Returns the numeric identifier assigned by the motion layer.
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

/// Controls how a newly started motion group interacts with existing groups.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MotionPolicy {
    /// Cancels all active and queued groups before starting the new group.
    Replace,
    /// Runs the new group alongside currently active groups.
    #[default]
    Concurrent,
    /// Waits until every active group has completed before starting.
    Queue,
}

/// Events emitted by [`MotionLayer`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MotionEvent {
    /// A group reached its shared arrival time.
    Completed(MotionId),
    /// A group was explicitly cancelled or replaced.
    Cancelled(MotionId),
}

/// A reusable easing function for coordinated motion.
#[derive(Clone)]
pub struct MotionEasing(Rc<dyn Fn(f32) -> f32>);

impl MotionEasing {
    /// Creates an easing function from a normalized progress callback.
    pub fn custom(easing: impl Fn(f32) -> f32 + 'static) -> Self {
        Self(Rc::new(easing))
    }

    /// Moves at a constant rate.
    pub fn linear() -> Self {
        Self::custom(|progress| progress)
    }

    /// Uses a cubic curve with zero velocity at both endpoints.
    pub fn smoothstep() -> Self {
        Self::custom(|progress| progress * progress * (3.0 - 2.0 * progress))
    }

    /// Uses a quintic curve with zero velocity and acceleration at both endpoints.
    pub fn smootherstep() -> Self {
        Self::custom(|progress| {
            progress * progress * progress * (progress * (progress * 6.0 - 15.0) + 10.0)
        })
    }

    /// Starts slowly and accelerates toward the destination.
    pub fn ease_in_cubic() -> Self {
        Self::custom(|progress| progress.powi(3))
    }

    /// Starts quickly and decelerates toward the destination.
    pub fn ease_out_cubic() -> Self {
        Self::custom(|progress| 1.0 - (1.0 - progress).powi(3))
    }

    fn sample(&self, progress: f32) -> f32 {
        let progress = progress.clamp(0.0, 1.0);
        if progress == 0.0 || progress == 1.0 {
            progress
        } else {
            (self.0)(progress)
        }
    }
}

impl Default for MotionEasing {
    fn default() -> Self {
        Self::smootherstep()
    }
}

/// Describes the spatial path followed by one motion item.
#[derive(Clone)]
pub struct MotionPath(Rc<dyn Fn(Point<Pixels>, Point<Pixels>, f32) -> Point<Pixels>>);

impl MotionPath {
    /// Creates a custom normalized path sampler.
    pub fn custom(
        sample: impl Fn(Point<Pixels>, Point<Pixels>, f32) -> Point<Pixels> + 'static,
    ) -> Self {
        Self(Rc::new(sample))
    }

    /// Follows the direct line between the start and destination.
    pub fn linear() -> Self {
        Self::custom(lerp_point)
    }

    /// Follows a quadratic Bezier curve through an absolute control point.
    pub fn quadratic(control: Point<Pixels>) -> Self {
        Self::custom(move |start, target, progress| {
            let left = lerp_point(start, control, progress);
            let right = lerp_point(control, target, progress);
            lerp_point(left, right, progress)
        })
    }

    /// Follows an arc whose midpoint is displaced perpendicular to the direct path.
    pub fn arc(bend: Pixels) -> Self {
        Self::custom(move |start, target, progress| {
            let dx = f32::from(target.x - start.x);
            let dy = f32::from(target.y - start.y);
            let length = (dx * dx + dy * dy).sqrt();
            if length <= f32::EPSILON {
                return start;
            }
            let midpoint = lerp_point(start, target, 0.5);
            let control = point(
                midpoint.x + bend * (-dy / length),
                midpoint.y + bend * (dx / length),
            );
            let left = lerp_point(start, control, progress);
            let right = lerp_point(control, target, progress);
            lerp_point(left, right, progress)
        })
    }

    fn sample(&self, start: Point<Pixels>, target: Point<Pixels>, progress: f32) -> Point<Pixels> {
        if progress == 0.0 {
            start
        } else if progress == 1.0 {
            target
        } else {
            (self.0)(start, target, progress)
        }
    }
}

impl Default for MotionPath {
    fn default() -> Self {
        Self::linear()
    }
}

/// Values supplied to an item's element builder for the current frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MotionFrame {
    /// Identifier of the coordinated group being rendered.
    pub id: MotionId,
    /// Index of the item inside its group.
    pub item_index: usize,
    /// Position where the item began.
    pub start: Point<Pixels>,
    /// Position where the item will arrive.
    pub target: Point<Pixels>,
    /// Current sampled position along the configured path.
    pub position: Point<Pixels>,
    /// Current position relative to `start`.
    pub offset: Point<Pixels>,
    /// Progress after applying the item's delay but before easing.
    pub linear_progress: f32,
    /// Progress after applying the configured easing function.
    pub progress: f32,
    /// Time elapsed since the entire group started.
    pub elapsed: Duration,
}

impl MotionFrame {
    /// Returns a GPU translation suitable for SVG and image-like elements.
    ///
    /// Place the element at `start`, then apply this transformation to avoid
    /// changing its layout position on each frame.
    pub fn translation(self) -> Transformation {
        Transformation::translate(self.offset)
    }

    /// Returns a convenient fade-out opacity in the `0..=1` range.
    pub fn fade_out(self) -> f32 {
        (1.0 - self.linear_progress).clamp(0.0, 1.0)
    }
}

type MotionElementBuilder = Rc<dyn Fn(MotionFrame) -> AnyElement>;

/// One independently rendered element participating in coordinated motion.
pub struct MotionItem {
    start: Point<Pixels>,
    target: Point<Pixels>,
    delay: Duration,
    path: MotionPath,
    render: MotionElementBuilder,
}

impl MotionItem {
    /// Creates an item with arbitrary GPUI element output.
    ///
    /// The callback is evaluated only by the independent [`MotionLayer`] view.
    pub fn new<E>(
        start: Point<Pixels>,
        target: Point<Pixels>,
        render: impl Fn(MotionFrame) -> E + 'static,
    ) -> Self
    where
        E: IntoElement + 'static,
    {
        Self {
            start,
            target,
            delay: Duration::ZERO,
            path: MotionPath::default(),
            render: Rc::new(move |frame| render(frame).into_any_element()),
        }
    }

    /// Delays this item's departure without changing the group's arrival time.
    pub fn delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    /// Selects the spatial path followed by this item.
    pub fn path(mut self, path: MotionPath) -> Self {
        self.path = path;
        self
    }
}

/// Configuration shared by every item in one coordinated group.
#[derive(Clone)]
pub struct MotionOptions {
    duration: Duration,
    easing: MotionEasing,
    policy: MotionPolicy,
}

impl MotionOptions {
    /// Creates options with the supplied shared arrival duration.
    pub fn new(duration: Duration) -> Self {
        Self {
            duration,
            ..Self::default()
        }
    }

    /// Sets the temporal easing applied to every item.
    pub fn easing(mut self, easing: MotionEasing) -> Self {
        self.easing = easing;
        self
    }

    /// Sets how this group interacts with groups already in the layer.
    pub fn policy(mut self, policy: MotionPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Returns the group's shared duration.
    pub fn duration(&self) -> Duration {
        self.duration
    }

    /// Returns the group's start policy.
    pub fn start_policy(&self) -> MotionPolicy {
        self.policy
    }
}

impl Default for MotionOptions {
    fn default() -> Self {
        Self {
            duration: Duration::from_millis(700),
            easing: MotionEasing::default(),
            policy: MotionPolicy::default(),
        }
    }
}

struct MotionBatch {
    id: MotionId,
    items: Vec<MotionItem>,
    options: MotionOptions,
    started_at: Option<Instant>,
}

/// An independently invalidated view for coordinated multi-element motion.
///
/// Store this as an `Entity<MotionLayer>` and add that entity as a child of a
/// relatively positioned container. Animation frames then invalidate this
/// small subtree instead of the surrounding application view.
/// Item builders may either position arbitrary elements with `frame.position`,
/// or place sprite-backed elements at `frame.start` and apply
/// [`MotionFrame::translation`] for a GPU-only translation.
#[derive(Default)]
pub struct MotionLayer {
    active: Vec<MotionBatch>,
    queued: VecDeque<MotionBatch>,
    next_id: u64,
}

impl MotionLayer {
    /// Creates an empty motion layer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts a coordinated group and returns its identifier.
    ///
    /// Every item uses the same total duration. Per-item delays shorten that
    /// item's travel interval, so all items still arrive together.
    pub fn start(
        &mut self,
        items: impl IntoIterator<Item = MotionItem>,
        options: MotionOptions,
        cx: &mut Context<Self>,
    ) -> Option<MotionId> {
        let items = items.into_iter().collect::<Vec<_>>();
        if items.is_empty() {
            return None;
        }

        self.next_id = self.next_id.wrapping_add(1);
        let id = MotionId(self.next_id);
        let mut batch = MotionBatch {
            id,
            items,
            options,
            started_at: None,
        };

        match batch.options.policy {
            MotionPolicy::Replace => {
                self.cancel_all(cx);
                batch.started_at = Some(Instant::now());
                self.active.push(batch);
            }
            MotionPolicy::Concurrent => {
                batch.started_at = Some(Instant::now());
                self.active.push(batch);
            }
            MotionPolicy::Queue => {
                if self.active.is_empty() {
                    batch.started_at = Some(Instant::now());
                    self.active.push(batch);
                } else {
                    self.queued.push_back(batch);
                }
            }
        }
        cx.notify();
        Some(id)
    }

    /// Cancels an active or queued group.
    pub fn cancel(&mut self, id: MotionId, cx: &mut Context<Self>) -> bool {
        let active_len = self.active.len();
        let queued_len = self.queued.len();
        self.active.retain(|batch| batch.id != id);
        self.queued.retain(|batch| batch.id != id);
        let cancelled = self.active.len() != active_len || self.queued.len() != queued_len;
        if cancelled {
            cx.emit(MotionEvent::Cancelled(id));
            self.start_next_queued(Instant::now());
            cx.notify();
        }
        cancelled
    }

    /// Cancels every active and queued group.
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.cancel_all(cx);
        cx.notify();
    }

    /// Returns whether the layer has active or queued work.
    pub fn is_animating(&self) -> bool {
        !self.active.is_empty() || !self.queued.is_empty()
    }

    /// Returns the number of groups currently moving.
    pub fn active_group_count(&self) -> usize {
        self.active.len()
    }

    /// Returns the number of groups waiting for active work to finish.
    pub fn queued_group_count(&self) -> usize {
        self.queued.len()
    }

    fn cancel_all(&mut self, cx: &mut Context<Self>) {
        for batch in self.active.drain(..).chain(self.queued.drain(..)) {
            cx.emit(MotionEvent::Cancelled(batch.id));
        }
    }

    fn start_next_queued(&mut self, now: Instant) -> bool {
        if !self.active.is_empty() {
            return false;
        }
        let Some(mut batch) = self.queued.pop_front() else {
            return false;
        };
        batch.started_at = Some(now);
        self.active.push(batch);
        true
    }
}

impl EventEmitter<MotionEvent> for MotionLayer {}

impl Render for MotionLayer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
        self.start_next_queued(now);

        let mut elements = Vec::new();
        let mut completed = Vec::new();
        for batch in &self.active {
            let started_at = batch
                .started_at
                .expect("active motion must have a start time");
            let elapsed = now.saturating_duration_since(started_at);
            let complete = elapsed >= batch.options.duration;
            for (item_index, item) in batch.items.iter().enumerate() {
                let linear_progress = item_progress(elapsed, batch.options.duration, item.delay);
                let progress = batch.options.easing.sample(linear_progress);
                let position = item.path.sample(item.start, item.target, progress);
                let frame = MotionFrame {
                    id: batch.id,
                    item_index,
                    start: item.start,
                    target: item.target,
                    position,
                    offset: point(position.x - item.start.x, position.y - item.start.y),
                    linear_progress,
                    progress,
                    elapsed,
                };
                elements.push((item.render)(frame));
            }
            if complete {
                completed.push(batch.id);
            }
        }

        if !completed.is_empty() {
            self.active.retain(|batch| !completed.contains(&batch.id));
            for id in completed {
                cx.emit(MotionEvent::Completed(id));
            }
            self.start_next_queued(now);
            // Draw one cleanup frame so completed elements cannot remain in the retained scene.
            window.request_animation_frame();
        } else if !self.active.is_empty() {
            window.request_animation_frame();
        }

        div()
            .id("gpui-motion-layer")
            .absolute()
            .inset_0()
            .children(elements)
    }
}

fn item_progress(elapsed: Duration, duration: Duration, delay: Duration) -> f32 {
    if elapsed >= duration {
        return 1.0;
    }
    let delay = delay.min(duration);
    if elapsed <= delay {
        return 0.0;
    }
    let travel_duration = duration.saturating_sub(delay);
    if travel_duration.is_zero() {
        return 1.0;
    }
    elapsed.saturating_sub(delay).as_secs_f32() / travel_duration.as_secs_f32()
}

fn lerp_point(start: Point<Pixels>, target: Point<Pixels>, progress: f32) -> Point<Pixels> {
    point(
        start.x + (target.x - start.x) * progress,
        start.y + (target.y - start.y) * progress,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;

    #[test]
    fn delayed_items_still_share_the_arrival_time() {
        let duration = Duration::from_millis(800);
        assert_eq!(item_progress(duration, duration, Duration::ZERO), 1.0);
        assert_eq!(
            item_progress(duration, duration, Duration::from_millis(300)),
            1.0
        );
        assert_eq!(
            item_progress(
                Duration::from_millis(300),
                duration,
                Duration::from_millis(300)
            ),
            0.0
        );
        assert!(
            (item_progress(
                Duration::from_millis(550),
                duration,
                Duration::from_millis(300)
            ) - 0.5)
                .abs()
                < f32::EPSILON
        );
    }

    #[test]
    fn smootherstep_has_exact_endpoints() {
        let easing = MotionEasing::smootherstep();
        assert_eq!(easing.sample(0.0), 0.0);
        assert_eq!(easing.sample(1.0), 1.0);
        assert!((easing.sample(0.5) - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn paths_have_exact_endpoints() {
        let start = point(px(10.0), px(20.0));
        let target = point(px(110.0), px(80.0));
        for path in [
            MotionPath::linear(),
            MotionPath::quadratic(point(px(50.0), px(-30.0))),
            MotionPath::arc(px(40.0)),
        ] {
            assert_eq!(path.sample(start, target, 0.0), start);
            assert_eq!(path.sample(start, target, 1.0), target);
        }
    }
}
