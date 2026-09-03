use std::{cell::RefCell, rc::Rc, time::Duration};

use gpui::{
    App, Bounds, DispatchPhase, Element, ElementId, GlobalElementId, Hitbox, HitboxBehavior,
    HitboxId, InspectorElementId, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Point, Refineable as _, Style, StyleRefinement, Styled, Window,
};

type ClickCallback = Rc<dyn Fn(Point<gpui::Pixels>, Bounds<gpui::Pixels>, &mut Window, &mut App)>;
type RangeCallback = Rc<
    dyn Fn(
        Point<gpui::Pixels>,
        Point<gpui::Pixels>,
        Bounds<gpui::Pixels>,
        LongPressPhase,
        &mut Window,
        &mut App,
    ),
>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LongPressPhase {
    Start,
    Update,
    End,
}

pub(super) struct TimelineInteraction {
    id: ElementId,
    click_callback: ClickCallback,
    range_callback: RangeCallback,
    long_press_delay: Duration,
    style: StyleRefinement,
}

#[derive(Default)]
struct PointerState {
    pressed: bool,
    moved_before_long_press: bool,
    long_press_fired: bool,
    generation: u64,
    origin: Point<gpui::Pixels>,
    last_position: Point<gpui::Pixels>,
    capture: Option<HitboxId>,
}

pub(super) struct LayoutState {
    style: Style,
    pointer: Rc<RefCell<PointerState>>,
}

pub(super) struct PrepaintState {
    hitbox: Hitbox,
    pointer: Rc<RefCell<PointerState>>,
}

impl TimelineInteraction {
    pub fn new(
        id: impl Into<ElementId>,
        click_callback: impl Fn(Point<gpui::Pixels>, Bounds<gpui::Pixels>, &mut Window, &mut App)
        + 'static,
        range_callback: impl Fn(
            Point<gpui::Pixels>,
            Point<gpui::Pixels>,
            Bounds<gpui::Pixels>,
            LongPressPhase,
            &mut Window,
            &mut App,
        ) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            click_callback: Rc::new(click_callback),
            range_callback: Rc::new(range_callback),
            long_press_delay: Duration::from_millis(350),
            style: StyleRefinement::default(),
        }
    }

    pub fn long_press_delay(mut self, delay: Duration) -> Self {
        self.long_press_delay = delay;
        self
    }
}

impl IntoElement for TimelineInteraction {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TimelineInteraction {
    type RequestLayoutState = LayoutState;
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        let pointer = window.with_element_state(
            id.expect("timeline interaction has an element id"),
            |state: Option<Rc<RefCell<PointerState>>>, _| {
                let state = state.unwrap_or_default();
                (state.clone(), state)
            },
        );
        let mut style = Style::default();
        style.refine(&self.style);
        let layout_id = window.request_layout(style.clone(), [], cx);
        (layout_id, LayoutState { style, pointer })
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<gpui::Pixels>,
        request: &mut Self::RequestLayoutState,
        window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let previous_capture = request.pointer.borrow().capture;
        if previous_capture.is_some_and(|capture| window.captured_hitbox() == Some(capture)) {
            window.capture_pointer(hitbox.id);
        }
        request.pointer.borrow_mut().capture = Some(hitbox.id);
        PrepaintState {
            hitbox,
            pointer: request.pointer.clone(),
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<gpui::Pixels>,
        request: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let hitbox = prepaint.hitbox.clone();
        let pointer = prepaint.pointer.clone();
        let click_callback = self.click_callback.clone();
        let range_callback = self.range_callback.clone();
        let long_press_delay = self.long_press_delay;

        request.style.paint(bounds, window, cx, move |window, _| {
            let down_hitbox = hitbox.clone();
            let down_pointer = pointer.clone();
            let down_callback = range_callback.clone();
            window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble
                    || event.button != MouseButton::Left
                    || !down_hitbox.is_hovered(window)
                {
                    return;
                }
                let generation = {
                    let mut state = down_pointer.borrow_mut();
                    state.pressed = true;
                    state.moved_before_long_press = false;
                    state.long_press_fired = false;
                    state.generation = state.generation.wrapping_add(1);
                    state.origin = event.position;
                    state.last_position = event.position;
                    state.generation
                };
                window.capture_pointer(down_hitbox.id);
                let pointer = down_pointer.clone();
                let callback = down_callback.clone();
                let bounds = down_hitbox.bounds;
                window
                    .spawn(cx, async move |cx| {
                        cx.background_executor().timer(long_press_delay).await;
                        let origin = {
                            let mut state = pointer.borrow_mut();
                            if !state.pressed
                                || state.moved_before_long_press
                                || state.generation != generation
                            {
                                return;
                            }
                            state.long_press_fired = true;
                            state.origin
                        };
                        _ = cx.update(|window, cx| {
                            callback(origin, origin, bounds, LongPressPhase::Start, window, cx)
                        });
                    })
                    .detach();
                cx.stop_propagation();
            });

            let move_pointer = pointer.clone();
            let move_callback = range_callback.clone();
            let move_hitbox = hitbox.clone();
            let move_bounds = hitbox.bounds;
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                if phase != DispatchPhase::Capture
                    || !event.dragging()
                    || window.captured_hitbox() != Some(move_hitbox.id)
                {
                    return;
                }
                let action = {
                    let mut state = move_pointer.borrow_mut();
                    if !state.pressed || state.last_position == event.position {
                        None
                    } else if state.long_press_fired {
                        state.last_position = event.position;
                        Some(state.origin)
                    } else {
                        state.last_position = event.position;
                        let x = f32::from(event.position.x - state.origin.x);
                        let y = f32::from(event.position.y - state.origin.y);
                        if x * x + y * y > 144. {
                            state.moved_before_long_press = true;
                        }
                        None
                    }
                };
                if let Some(origin) = action {
                    move_callback(
                        origin,
                        event.position,
                        move_bounds,
                        LongPressPhase::Update,
                        window,
                        cx,
                    );
                    cx.stop_propagation();
                }
            });

            let up_pointer = pointer.clone();
            let up_hitbox = hitbox.clone();
            window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                if phase != DispatchPhase::Capture
                    || event.button != MouseButton::Left
                    || window.captured_hitbox() != Some(up_hitbox.id)
                {
                    return;
                }
                let (origin, long_press_fired, should_click) = {
                    let mut state = up_pointer.borrow_mut();
                    let result = (
                        state.origin,
                        state.long_press_fired,
                        state.pressed
                            && !state.moved_before_long_press
                            && !state.long_press_fired
                            && up_hitbox.bounds.contains(&event.position),
                    );
                    state.pressed = false;
                    state.generation = state.generation.wrapping_add(1);
                    result
                };
                window.release_pointer();
                if long_press_fired {
                    range_callback(
                        origin,
                        event.position,
                        up_hitbox.bounds,
                        LongPressPhase::End,
                        window,
                        cx,
                    );
                    cx.stop_propagation();
                } else if should_click {
                    click_callback(event.position, up_hitbox.bounds, window, cx);
                    cx.stop_propagation();
                }
            });
        });
    }
}

impl Styled for TimelineInteraction {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}
