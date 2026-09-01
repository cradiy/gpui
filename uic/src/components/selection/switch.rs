use std::rc::Rc;

use gpui::{
    AccessibleAction, App, ElementId, IntoElement, RenderOnce, Role, SharedString, StyleRefinement,
    Styled, Window, div, prelude::*, px,
};

use super::SwitchAppearance;

type ChangeCallback = Rc<dyn Fn(bool, &mut Window, &mut App)>;

/// A controlled boolean switch. Compose labels outside the track when needed.
#[derive(IntoElement)]
pub struct Switch {
    id: ElementId,
    checked: bool,
    disabled: bool,
    label: Option<SharedString>,
    on_change: Option<ChangeCallback>,
    appearance: SwitchAppearance,
    style: StyleRefinement,
}

impl Switch {
    pub fn new(id: impl Into<ElementId>, checked: bool) -> Self {
        Self {
            id: id.into(),
            checked,
            disabled: false,
            label: None,
            on_change: None,
            appearance: SwitchAppearance::default(),
            style: StyleRefinement::default()
                .w(px(40.))
                .h(px(22.))
                .p(px(2.))
                .rounded_full()
                .border_1()
                .border_color(gpui::black().opacity(0.1)),
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn on_change(mut self, callback: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(callback));
        self
    }

    pub fn appearance(mut self, appearance: SwitchAppearance) -> Self {
        self.appearance = appearance;
        self
    }
}

impl RenderOnce for Switch {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let next_value = !self.checked;
        let track = if self.checked {
            self.appearance.on_track
        } else {
            self.appearance.off_track
        };
        let hover_track = if self.checked {
            self.appearance.hover_on_track
        } else {
            self.appearance.hover_off_track
        };
        let click_callback = self.on_change.clone();
        let key_callback = self.on_change.clone();
        let action_callback = self.on_change.clone();

        let mut element = div()
            .id(self.id)
            .debug_selector(|| "uic-switch".to_string())
            .focusable()
            .tab_stop(!self.disabled)
            .role(Role::Switch)
            .aria_toggled(self.checked.into())
            .when_some(self.label, |this, label| this.aria_label(label))
            .flex()
            .items_center()
            .when(self.checked, |this| this.justify_end())
            .when(!self.checked, |this| this.justify_start())
            .bg(track)
            .opacity(if self.disabled {
                self.appearance.disabled_opacity
            } else {
                1.0
            })
            .child(
                div()
                    .flex_none()
                    .size(self.appearance.thumb_size)
                    .rounded_full()
                    .bg(self.appearance.thumb)
                    .shadow_md(),
            );
        element.style().refine(&self.style);
        if !self.disabled {
            element = element
                .cursor_pointer()
                .hover(|style| style.bg(hover_track))
                .when_some(click_callback, |this, callback| {
                    this.on_click(move |_, window, cx| callback(next_value, window, cx))
                })
                .when_some(key_callback, |this, callback| {
                    this.on_key_down(move |event, window, cx| {
                        if event.keystroke.key == "space" {
                            callback(next_value, window, cx);
                            cx.stop_propagation();
                        }
                    })
                })
                .when_some(action_callback, |this, callback| {
                    this.on_a11y_action(AccessibleAction::Click, move |_, window, cx| {
                        callback(next_value, window, cx)
                    })
                });
        }
        element.focus_visible(move |style| style.border_color(self.appearance.focus_ring))
    }
}

impl Styled for Switch {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}
