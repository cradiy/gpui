use gpui::{
    Anchor, AnyElement, App, CursorStyle, Entity, Focusable, IntoElement, MouseButton, Pixels,
    Refineable as _, RenderOnce, StyleRefinement, Styled, Window, anchored, deferred,
    deferred_overlay, div, point, prelude::*, px,
};

use crate::components::overlay_anchor::{TriggerAnchor, resolve_overlay};

use super::{DropdownPlacement, DropdownState};
use std::rc::Rc;

type MenuRenderer = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;

enum MenuContent {
    Element(AnyElement),
    Renderer(MenuRenderer),
}

#[derive(IntoElement)]
pub struct Dropdown {
    state: Entity<DropdownState>,
    trigger: Option<AnyElement>,
    menu: Option<MenuContent>,
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

    /// Sets a menu element constructed with the containing view.
    /// Use [`Self::menu_with`] for an independently rendered menu.
    pub fn menu(mut self, menu: impl IntoElement) -> Self {
        self.menu = Some(MenuContent::Element(menu.into_any_element()));
        self
    }

    /// Builds the menu independently of its trigger on each drawn frame while open.
    /// Use this with cached, transformed views so repositioning the menu does not
    /// require rebuilding the containing view. Read changing state in the renderer.
    pub fn menu_with<E: IntoElement>(
        mut self,
        render: impl Fn(&mut Window, &mut App) -> E + 'static,
    ) -> Self {
        self.menu = Some(MenuContent::Renderer(Rc::new(move |window, cx| {
            render(window, cx).into_any_element()
        })));
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
            let trigger_bounds = trigger_bounds.clone();
            let render_menu = move |menu: Option<AnyElement>, window: &mut Window| {
                let bounds = trigger_bounds.bounds(window)?;
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
                    .children(menu);
                positioned.style().refine(&menu_style);
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
            };
            match self.menu {
                Some(MenuContent::Renderer(render)) => deferred_overlay(move |window, cx| {
                    let menu = render(window, cx);
                    render_menu(Some(menu), window)
                }),
                Some(MenuContent::Element(menu)) => deferred(resolve_overlay(move |window, _| {
                    render_menu(Some(menu), window)
                })),
                None => deferred(resolve_overlay(move |window, _| render_menu(None, window))),
            }
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
