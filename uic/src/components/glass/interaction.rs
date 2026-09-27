use std::{cell::Cell, rc::Rc, time::Instant};

use gpui::{
    Along, App, Axis, Bounds, DispatchPhase, Entity, HitboxBehavior, HitboxId, IntoElement,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Window, canvas,
    prelude::*, px,
};

use super::motion::Motion;

pub(super) type ChangeCallback<T> = Rc<dyn Fn(T, &mut Window, &mut App)>;

pub(super) struct OptionGeometry<T> {
    pub value: T,
    pub disabled: bool,
    pub bounds: Cell<Option<Bounds<Pixels>>>,
}

pub(super) struct State {
    pub motion: Motion,
    pub axis: Axis,
    capture: Option<HitboxId>,
    press: Option<Press>,
}

struct Press {
    start: Point<Pixels>,
    bounds: Bounds<Pixels>,
    dragging: bool,
}

impl State {
    pub fn new() -> Self {
        Self {
            motion: Motion::default(),
            axis: Axis::Horizontal,
            capture: None,
            press: None,
        }
    }

    pub fn cancel(&mut self, window: &mut Window) {
        if self.capture.is_some() && window.captured_hitbox() == self.capture {
            window.release_pointer();
        }
        self.press = None;
        self.motion.cancel_press();
        self.motion.pointer = None;
    }

    fn move_lens<T>(
        &mut self,
        position: Point<Pixels>,
        root: Bounds<Pixels>,
        options: &[OptionGeometry<T>],
    ) {
        let Some(press) = &mut self.press else { return };
        let axis = self.axis;
        let delta = position.along(axis) - press.start.along(axis);
        press.dragging |= delta.abs() > px(4.);
        self.motion.pointer = Some(position);
        if !press.dragging {
            return;
        }
        let mut centers = options.iter().filter(|o| !o.disabled).filter_map(|o| {
            o.bounds
                .get()
                .map(|b| b.center().along(axis) - root.origin.along(axis))
        });
        let Some(first) = centers.next() else { return };
        let (min, max) = centers.fold((first, first), |(min, max), x| (min.min(x), max.max(x)));
        let mut bounds = press.bounds;
        bounds.origin = bounds.origin.apply_along(axis, |_| {
            (bounds.center().along(axis) + delta).clamp(min, max) - bounds.size.along(axis) / 2.
        });
        self.motion.drag_bounds = Some(bounds);
    }
}

pub(super) fn interaction<T: Clone + PartialEq + 'static>(
    state: Entity<State>,
    options: Rc<Vec<OptionGeometry<T>>>,
    selected: T,
    root_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    callback: Option<ChangeCallback<T>>,
    enabled: bool,
) -> impl IntoElement {
    let prepaint = state.clone();
    canvas(
        move |bounds, window, cx| {
            let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
            prepaint.update(cx, |state, _| {
                if !enabled || !window.is_window_active() {
                    state.cancel(window);
                } else if state.capture.is_some() && window.captured_hitbox() == state.capture {
                    window.capture_pointer(hitbox.id);
                } else if state.press.is_some() {
                    state.cancel(window);
                }
                state.capture = Some(hitbox.id);
            });
            hitbox
        },
        move |bounds, hitbox, window, _| {
            let bounds = root_bounds.get().unwrap_or(bounds);
            if !enabled {
                return;
            }
            let down_state = state.clone();
            let down_options = options.clone();
            let down_hitbox = hitbox.clone();
            let down_selected = selected.clone();
            let down_callback = callback.clone();
            window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble
                    || event.button != MouseButton::Left
                    || !down_hitbox.is_hovered(window)
                {
                    return;
                }
                let Some((index, option_bounds)) = down_options
                    .iter()
                    .enumerate()
                    .filter(|(_, o)| !o.disabled)
                    .find_map(|(i, o)| {
                        o.bounds
                            .get()
                            .filter(|b| b.contains(&event.position))
                            .map(|b| (i, b))
                    })
                else {
                    return;
                };
                window.capture_pointer(down_hitbox.id);
                down_state.update(cx, |state, cx| {
                    state.motion.begin_press(Instant::now());
                    state.motion.pointer = Some(event.position);
                    let target =
                        Bounds::new(option_bounds.origin - bounds.origin, option_bounds.size);
                    let local = if down_options[index].value == down_selected {
                        state.motion.current_bounds().unwrap_or(target)
                    } else {
                        target
                    };
                    state.press = Some(Press {
                        start: event.position,
                        bounds: local,
                        dragging: false,
                    });
                    cx.notify();
                });
                if down_options[index].value != down_selected
                    && let Some(callback) = &down_callback
                {
                    callback(down_options[index].value.clone(), window, cx);
                }
                cx.stop_propagation();
            });
            let move_state = state.clone();
            let move_options = options.clone();
            let move_hitbox = hitbox.clone();
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                if phase == DispatchPhase::Capture
                    && event.dragging()
                    && window.captured_hitbox() == Some(move_hitbox.id)
                {
                    move_state.update(cx, |state, cx| {
                        state.move_lens(event.position, bounds, &move_options);
                        cx.notify();
                    });
                    cx.stop_propagation();
                }
            });
            window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                if phase != DispatchPhase::Capture
                    || event.button != MouseButton::Left
                    || window.captured_hitbox() != Some(hitbox.id)
                {
                    return;
                }
                window.release_pointer();
                let value = state.update(cx, |state, cx| {
                    state.move_lens(event.position, bounds, &options);
                    let press = state.press.take();
                    let axis = state.axis;
                    let center = state
                        .motion
                        .drag_bounds
                        .map(|b| b.center().along(axis) + bounds.origin.along(axis));
                    state.motion.drag_bounds = None;
                    state.motion.pressed = false;
                    cx.notify();
                    let press = press?;
                    if !press.dragging {
                        return None;
                    }
                    let center = center?;
                    let option = options
                        .iter()
                        .filter(|o| !o.disabled && o.bounds.get().is_some())
                        .min_by(|a, b| {
                            let distance = |o: &OptionGeometry<T>| {
                                f32::from(
                                    (o.bounds.get().unwrap().center().along(axis) - center).abs(),
                                )
                            };
                            distance(a).total_cmp(&distance(b))
                        })?;
                    (option.value != selected).then(|| option.value.clone())
                });
                if let Some(value) = value
                    && let Some(callback) = &callback
                {
                    callback(value, window, cx);
                }
                cx.stop_propagation();
            });
        },
    )
    .absolute()
    .inset_0()
    .size_full()
}
