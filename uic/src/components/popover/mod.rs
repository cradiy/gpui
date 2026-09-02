use std::{cell::Cell, rc::Rc};

use gpui::{
    Anchor, AnyElement, App, Bounds, Context, Entity, EventEmitter, FocusHandle, Focusable,
    IntoElement, MouseButton, Pixels, Point, RenderOnce, Role, SharedString, StyleRefinement,
    Styled, Subscription, Window, anchored, canvas, deferred, div, point, prelude::*, px,
};

type BoundsTracker = Rc<Cell<Option<Bounds<Pixels>>>>;
type ContentRenderer = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;

/// Where the popover should appear relative to its trigger.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PopoverPlacement {
    TopStart,
    Top,
    TopEnd,
    #[default]
    BottomStart,
    Bottom,
    BottomEnd,
    LeftStart,
    Left,
    LeftEnd,
    RightStart,
    Right,
    RightEnd,
}

/// How the popover should remain within the viewport.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PopoverCollision {
    /// Flip to the opposite side when it fits, then shift inside the viewport.
    #[default]
    FlipAndShift,
    /// Keep the requested side and shift inside the viewport margin.
    Shift,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopoverDismissReason {
    Programmatic,
    Trigger,
    OutsideClick,
    Escape,
    FocusLost,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopoverEvent {
    Opened,
    Dismissed(PopoverDismissReason),
}

/// Persistent open/focus state shared by a [`Popover`] across renders.
pub struct PopoverState {
    open: bool,
    focus_handle: FocusHandle,
    previous_focus: Option<FocusHandle>,
    trigger_bounds: BoundsTracker,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<PopoverEvent> for PopoverState {}

impl PopoverState {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        let blur_subscription = cx.on_focus_out(&focus_handle, window, |state, _, window, cx| {
            state.dismiss(PopoverDismissReason::FocusLost, false, window, cx);
        });
        Self {
            open: false,
            focus_handle,
            previous_focus: None,
            trigger_bounds: Rc::new(Cell::new(None)),
            _subscriptions: vec![blur_subscription],
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open {
            return;
        }
        self.previous_focus = window.focused(cx);
        self.open = true;
        self.focus_handle.focus(window, cx);
        cx.emit(PopoverEvent::Opened);
        window.refresh();
        cx.notify();
    }

    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss(PopoverDismissReason::Programmatic, true, window, cx);
    }

    pub fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open {
            self.dismiss(PopoverDismissReason::Trigger, true, window, cx);
        } else {
            self.open(window, cx);
        }
    }

    pub fn dismiss(
        &mut self,
        reason: PopoverDismissReason,
        restore_focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.open {
            return;
        }
        self.open = false;
        if restore_focus && let Some(previous_focus) = self.previous_focus.take() {
            previous_focus.focus(window, cx);
        } else {
            self.previous_focus = None;
        }
        cx.emit(PopoverEvent::Dismissed(reason));
        window.refresh();
        cx.notify();
    }
}

impl Focusable for PopoverState {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// A trigger-anchored floating surface with shared dismissal and focus behavior.
#[derive(IntoElement)]
pub struct Popover {
    state: Entity<PopoverState>,
    trigger: Option<AnyElement>,
    content: Option<ContentRenderer>,
    placement: PopoverPlacement,
    collision: PopoverCollision,
    gap: Pixels,
    viewport_margin: Pixels,
    priority: usize,
    disabled: bool,
    close_on_escape: bool,
    close_on_outside: bool,
    label: Option<SharedString>,
    style: StyleRefinement,
}

impl Popover {
    pub fn new(state: &Entity<PopoverState>) -> Self {
        Self {
            state: state.clone(),
            trigger: None,
            content: None,
            placement: PopoverPlacement::default(),
            collision: PopoverCollision::default(),
            gap: px(6.),
            viewport_margin: px(8.),
            priority: 1_000,
            disabled: false,
            close_on_escape: true,
            close_on_outside: true,
            label: None,
            style: StyleRefinement::default()
                .min_w(px(160.))
                .max_w(px(420.))
                .p_3()
                .rounded(px(10.))
                .border_1()
                .border_color(gpui::black().opacity(0.12))
                .bg(gpui::white())
                .text_color(gpui::black())
                .shadow_lg(),
        }
    }

    pub fn trigger(mut self, trigger: impl IntoElement) -> Self {
        self.trigger = Some(trigger.into_any_element());
        self
    }

    pub fn content<E>(mut self, render: impl Fn(&mut Window, &mut App) -> E + 'static) -> Self
    where
        E: IntoElement,
    {
        self.content = Some(Rc::new(move |window, cx| {
            render(window, cx).into_any_element()
        }));
        self
    }

    pub fn placement(mut self, placement: PopoverPlacement) -> Self {
        self.placement = placement;
        self
    }

    pub fn collision(mut self, collision: PopoverCollision) -> Self {
        self.collision = collision;
        self
    }

    pub fn gap(mut self, gap: Pixels) -> Self {
        self.gap = gap.max(Pixels::ZERO);
        self
    }

    pub fn viewport_margin(mut self, margin: Pixels) -> Self {
        self.viewport_margin = margin.max(Pixels::ZERO);
        self
    }

    pub fn priority(mut self, priority: usize) -> Self {
        self.priority = priority;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn close_on_escape(mut self, close: bool) -> Self {
        self.close_on_escape = close;
        self
    }

    pub fn close_on_outside(mut self, close: bool) -> Self {
        self.close_on_outside = close;
        self
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }
}

impl RenderOnce for Popover {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state_id = self.state.entity_id();
        let (open, focus_handle, trigger_bounds) = {
            let state = self.state.read(cx);
            (
                state.is_open(),
                state.focus_handle(cx),
                state.trigger_bounds.clone(),
            )
        };

        let toggle_state = self.state.clone();
        let keyboard_state = self.state.clone();
        let trigger = div()
            .id(("uic-popover-trigger", state_id))
            .debug_selector(|| "uic-popover-trigger".to_string())
            .focusable()
            .tab_stop(!self.disabled)
            .role(Role::Button)
            .aria_expanded(open)
            .when_some(self.label, |this, label| this.aria_label(label))
            .when(!self.disabled, |this| {
                this.cursor_pointer()
                    .on_click(move |_, window, cx| {
                        toggle_state.update(cx, |state, cx| state.toggle(window, cx));
                        cx.stop_propagation();
                    })
                    .on_key_down(move |event, window, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            keyboard_state.update(cx, |state, cx| state.toggle(window, cx));
                            cx.stop_propagation();
                        }
                    })
            })
            .children(self.trigger)
            .child(bounds_tracker(trigger_bounds.clone()));

        let overlay = if open {
            match (trigger_bounds.get(), self.content) {
                (Some(bounds), Some(render)) => {
                    let content = render(window, cx);
                    let mut surface = div()
                        .id(("uic-popover-surface", state_id))
                        .debug_selector(|| "uic-popover-surface".to_string())
                        .occlude()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(content);
                    surface.style().refine(&self.style);

                    let (position, anchor) = placement(bounds, self.placement, self.gap);
                    let positioned = anchored().position(position).anchor(anchor).child(surface);
                    let positioned = match self.collision {
                        PopoverCollision::FlipAndShift => positioned,
                        PopoverCollision::Shift => {
                            positioned.snap_to_window_with_margin(self.viewport_margin)
                        }
                    };

                    let outside_state = self.state.clone();
                    let viewport = window.viewport_size();
                    let mut layer = div()
                        .id(("uic-popover-layer", state_id))
                        .absolute()
                        .left(-bounds.left())
                        .top(-bounds.top())
                        .w(viewport.width)
                        .h(viewport.height)
                        .child(positioned);
                    if self.close_on_outside {
                        layer = layer.on_mouse_down(MouseButton::Left, move |_, window, cx| {
                            outside_state.update(cx, |state, cx| {
                                state.dismiss(PopoverDismissReason::OutsideClick, true, window, cx)
                            });
                            cx.stop_propagation();
                        });
                    }
                    Some(deferred(layer).with_priority(self.priority))
                }
                _ => None,
            }
        } else {
            None
        };

        div()
            .relative()
            .track_focus(&focus_handle)
            .when(self.close_on_escape, |this| {
                this.on_key_down(move |event, window, cx| {
                    if event.keystroke.key == "escape" && self.state.read(cx).is_open() {
                        self.state.update(cx, |state, cx| {
                            state.dismiss(PopoverDismissReason::Escape, true, window, cx)
                        });
                        cx.stop_propagation();
                    }
                })
            })
            .child(trigger)
            .children(overlay)
    }
}

impl Styled for Popover {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

fn placement(
    bounds: Bounds<Pixels>,
    placement: PopoverPlacement,
    gap: Pixels,
) -> (Point<Pixels>, Anchor) {
    match placement {
        PopoverPlacement::TopStart => {
            (point(bounds.left(), bounds.top() - gap), Anchor::BottomLeft)
        }
        PopoverPlacement::Top => (
            point(bounds.center().x, bounds.top() - gap),
            Anchor::BottomCenter,
        ),
        PopoverPlacement::TopEnd => (
            point(bounds.right(), bounds.top() - gap),
            Anchor::BottomRight,
        ),
        PopoverPlacement::BottomStart => {
            (point(bounds.left(), bounds.bottom() + gap), Anchor::TopLeft)
        }
        PopoverPlacement::Bottom => (
            point(bounds.center().x, bounds.bottom() + gap),
            Anchor::TopCenter,
        ),
        PopoverPlacement::BottomEnd => (
            point(bounds.right(), bounds.bottom() + gap),
            Anchor::TopRight,
        ),
        PopoverPlacement::LeftStart => (point(bounds.left() - gap, bounds.top()), Anchor::TopRight),
        PopoverPlacement::Left => (
            point(bounds.left() - gap, bounds.center().y),
            Anchor::RightCenter,
        ),
        PopoverPlacement::LeftEnd => (
            point(bounds.left() - gap, bounds.bottom()),
            Anchor::BottomRight,
        ),
        PopoverPlacement::RightStart => {
            (point(bounds.right() + gap, bounds.top()), Anchor::TopLeft)
        }
        PopoverPlacement::Right => (
            point(bounds.right() + gap, bounds.center().y),
            Anchor::LeftCenter,
        ),
        PopoverPlacement::RightEnd => (
            point(bounds.right() + gap, bounds.bottom()),
            Anchor::BottomLeft,
        ),
    }
}

fn bounds_tracker(tracker: BoundsTracker) -> impl IntoElement {
    canvas(
        move |bounds, _, _| tracker.set(Some(bounds)),
        |_, _, _, _| {},
    )
    .absolute()
    .inset_0()
}

#[cfg(test)]
mod tests {
    use gpui::{Context, Modifiers, Render, TestAppContext, VisualTestContext, point, size};

    use super::*;

    struct TestPopover {
        state: Entity<PopoverState>,
        content_focus: FocusHandle,
    }

    impl Render for TestPopover {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let content_focus = self.content_focus.clone();
            div().size_full().p(px(80.)).child(
                Popover::new(&self.state)
                    .trigger(div().w(px(120.)).h(px(32.)).child("Open"))
                    .content(move |_, _| {
                        div()
                            .id("popover-content-focus")
                            .track_focus(&content_focus)
                            .tab_stop(true)
                            .role(Role::Button)
                            .w(px(180.))
                            .h(px(90.))
                            .child("Popover content")
                    }),
            )
        }
    }

    fn open_test_popover(cx: &mut TestAppContext) -> gpui::WindowHandle<TestPopover> {
        cx.open_window(size(px(420.), px(300.)), |window, cx| TestPopover {
            state: cx.new(|cx| PopoverState::new(window, cx)),
            content_focus: cx.focus_handle(),
        })
    }

    fn draw(
        window: &gpui::WindowHandle<TestPopover>,
        cx: &mut TestAppContext,
    ) -> VisualTestContext {
        let mut visual = VisualTestContext::from_window((*window).into(), cx);
        visual.update(|window, cx| {
            window.draw(cx).clear();
        });
        visual
    }

    #[gpui::test]
    fn trigger_escape_and_outside_click_control_visibility(cx: &mut TestAppContext) {
        let window = open_test_popover(cx);
        let mut visual = draw(&window, cx);
        let trigger = visual
            .debug_bounds("uic-popover-trigger")
            .expect("trigger should be rendered");

        visual.simulate_click(trigger.center(), Modifiers::default());
        visual.update(|window, cx| {
            window.draw(cx).clear();
        });
        window
            .update(&mut visual.cx, |view, _, cx| {
                assert!(view.state.read(cx).is_open());
            })
            .unwrap();
        assert!(visual.debug_bounds("uic-popover-surface").is_some());

        visual.simulate_keystrokes("escape");
        visual.update(|window, cx| {
            window.draw(cx).clear();
        });
        window
            .update(&mut visual.cx, |view, _, cx| {
                assert!(!view.state.read(cx).is_open());
            })
            .unwrap();

        visual.simulate_click(trigger.center(), Modifiers::default());
        visual.update(|window, cx| {
            window.draw(cx).clear();
        });
        visual.simulate_click(point(px(8.), px(8.)), Modifiers::default());
        window
            .update(&mut visual.cx, |view, _, cx| {
                assert!(!view.state.read(cx).is_open());
            })
            .unwrap();
    }

    #[gpui::test]
    fn focusing_interactive_content_keeps_the_popover_open(cx: &mut TestAppContext) {
        let window = open_test_popover(cx);
        let mut visual = draw(&window, cx);
        let trigger = visual
            .debug_bounds("uic-popover-trigger")
            .expect("trigger should be rendered");

        visual.simulate_click(trigger.center(), Modifiers::default());
        visual.update(|window, cx| {
            window.draw(cx).clear();
        });
        window
            .update(&mut visual.cx, |view, window, cx| {
                view.content_focus.focus(window, cx);
            })
            .unwrap();
        visual.update(|window, cx| {
            window.draw(cx).clear();
        });

        window
            .update(&mut visual.cx, |view, _, cx| {
                assert!(view.state.read(cx).is_open());
            })
            .unwrap();
    }
}
