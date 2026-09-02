use std::{cell::Cell, rc::Rc};

use gpui::{
    App, Bounds, DispatchPhase, Element, ElementId, GlobalElementId, Hitbox, HitboxBehavior,
    HitboxId, InspectorElementId, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Point, Refineable as _, Style, StyleRefinement, Styled, Window,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DragPhase {
    Start,
    Move,
    End,
}

type Callback = Rc<dyn Fn(Point<gpui::Pixels>, DragPhase, &mut Window, &mut App)>;

#[derive(Clone, Default)]
pub(super) struct CaptureToken(Rc<Cell<Option<HitboxId>>>);

pub(super) struct BadgeInteraction {
    capture: CaptureToken,
    callback: Callback,
    style: StyleRefinement,
}

impl BadgeInteraction {
    pub(super) fn new(
        capture: CaptureToken,
        callback: impl Fn(Point<gpui::Pixels>, DragPhase, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            capture,
            callback: Rc::new(callback),
            style: StyleRefinement::default(),
        }
    }
}

impl IntoElement for BadgeInteraction {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for BadgeInteraction {
    type RequestLayoutState = Style;
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.refine(&self.style);
        let layout_id = window.request_layout(style.clone(), [], cx);
        (layout_id, style)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<gpui::Pixels>,
        _style: &mut Style,
        window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        if let Some(previous) = self.capture.0.get()
            && window.captured_hitbox() == Some(previous)
        {
            window.capture_pointer(hitbox.id);
        }
        self.capture.0.set(Some(hitbox.id));
        hitbox
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<gpui::Pixels>,
        style: &mut Style,
        hitbox: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let down_hitbox = hitbox.clone();
        let down_callback = self.callback.clone();
        let move_hitbox = hitbox.clone();
        let move_callback = self.callback.clone();
        let up_hitbox = hitbox.clone();
        let up_callback = self.callback.clone();

        style.paint(bounds, window, cx, move |window, _| {
            window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                if phase == DispatchPhase::Bubble
                    && event.button == MouseButton::Left
                    && down_hitbox.is_hovered(window)
                {
                    window.capture_pointer(down_hitbox.id);
                    down_callback(event.position, DragPhase::Start, window, cx);
                    cx.stop_propagation();
                }
            });
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                if phase == DispatchPhase::Capture
                    && event.dragging()
                    && window.captured_hitbox() == Some(move_hitbox.id)
                {
                    move_callback(event.position, DragPhase::Move, window, cx);
                    cx.stop_propagation();
                }
            });
            window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                if phase == DispatchPhase::Capture
                    && event.button == MouseButton::Left
                    && window.captured_hitbox() == Some(up_hitbox.id)
                {
                    up_callback(event.position, DragPhase::End, window, cx);
                    cx.stop_propagation();
                }
            });
        });
    }
}

impl Styled for BadgeInteraction {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}
