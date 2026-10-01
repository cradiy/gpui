use gpui::{
    Anchor, AnyElement, App, CursorStyle, Entity, Focusable, IntoElement, MouseButton, Pixels,
    Refineable as _, RenderOnce, StyleRefinement, Styled, Window, anchored, deferred, div, point,
    prelude::*, px,
};

use crate::components::overlay_anchor::{TriggerAnchor, resolve_overlay};

use super::{DropdownPlacement, DropdownState};

#[derive(IntoElement)]
pub struct Dropdown {
    state: Entity<DropdownState>,
    trigger: Option<AnyElement>,
    menu: Option<AnyElement>,
    placement: DropdownPlacement,
    menu_gap: Pixels,
    priority: usize,
    style: StyleRefinement,
}

pub fn dropdown(state: &Entity<DropdownState>) -> Dropdown {
    Dropdown::new(state)
}

impl Dropdown {
    pub fn new(state: &Entity<DropdownState>) -> Self {
        Self {
            state: state.clone(),
            trigger: None,
            menu: None,
            placement: DropdownPlacement::default(),
            menu_gap: px(6.),
            priority: 100,
            style: StyleRefinement::default()
                .min_w(px(160.))
                .max_h(px(320.))
                .p(px(6.))
                .rounded(px(8.))
                .border(px(1.))
                .border_color(gpui::black().opacity(0.12))
                .bg(gpui::white()),
        }
    }

    pub fn trigger(mut self, trigger: impl IntoElement) -> Self {
        self.trigger = Some(trigger.into_any_element());
        self
    }

    pub fn menu(mut self, menu: impl IntoElement) -> Self {
        self.menu = Some(menu.into_any_element());
        self
    }

    pub fn placement(mut self, placement: DropdownPlacement) -> Self {
        self.placement = placement;
        self
    }

    pub fn menu_gap(mut self, gap: Pixels) -> Self {
        self.menu_gap = gap.max(px(0.));
        self
    }

    pub fn priority(mut self, priority: usize) -> Self {
        self.priority = priority;
        self
    }
}

impl RenderOnce for Dropdown {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let open = self.state.read(cx).is_open();
        let state_id = self.state.entity_id();
        let focus_handle = self.state.focus_handle(cx);
        let toggle_state = self.state.clone();
        let escape_state = self.state.clone();
        let menu_style = self.style.clone();
        let menu_gap = self.menu_gap;
        let trigger_bounds = TriggerAnchor::default();

        let menu = open.then(|| {
            let outside_state = self.state.clone();
            let outside_trigger = trigger_bounds.clone();
            let mut positioned = div()
                .id(("dropdown-menu", state_id))
                .debug_selector(|| "uic-dropdown-menu".to_string())
                .overflow_y_scroll()
                .occlude()
                .on_mouse_down_out(move |event, window, cx| {
                    if !outside_trigger.contains(event.position, window) {
                        outside_state.update(cx, |state, cx| state.close(window, cx));
                    }
                })
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .children(self.menu);
            positioned.style().refine(&menu_style);
            let trigger = trigger_bounds.clone();
            deferred(resolve_overlay(move |window, _| {
                let bounds = trigger.bounds(window)?;
                let (position, anchor) = match self.placement {
                    DropdownPlacement::BottomStart => (
                        point(bounds.left(), bounds.bottom() + menu_gap),
                        Anchor::TopLeft,
                    ),
                    DropdownPlacement::BottomEnd => (
                        point(bounds.right(), bounds.bottom() + menu_gap),
                        Anchor::TopRight,
                    ),
                    DropdownPlacement::TopStart => (
                        point(bounds.left(), bounds.top() - menu_gap),
                        Anchor::BottomLeft,
                    ),
                    DropdownPlacement::TopEnd => (
                        point(bounds.right(), bounds.top() - menu_gap),
                        Anchor::BottomRight,
                    ),
                };
                Some(
                    anchored()
                        .position(position)
                        .anchor(anchor)
                        .child(positioned)
                        .into_any_element(),
                )
            }))
            .with_priority(self.priority)
        });

        div()
            .relative()
            .track_focus(&focus_handle)
            .on_key_down(move |event, window, cx| {
                if event.keystroke.key == "escape" && escape_state.read(cx).is_open() {
                    escape_state.update(cx, |state, cx| state.close(window, cx));
                    cx.stop_propagation();
                }
            })
            .child(
                div()
                    .id(("dropdown-trigger", state_id))
                    .debug_selector(|| "uic-dropdown-trigger".to_string())
                    .cursor(CursorStyle::PointingHand)
                    .occlude()
                    .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                        toggle_state.update(cx, |state, cx| state.toggle(window, cx));
                        cx.stop_propagation();
                    })
                    .children(self.trigger),
            )
            .children(menu)
            .child(trigger_bounds.tracker())
    }
}

impl Styled for Dropdown {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}
