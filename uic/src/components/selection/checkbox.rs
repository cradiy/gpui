use std::rc::Rc;

use gpui::{
    AccessibleAction, AnyElement, App, ElementId, IntoElement, RenderOnce, Role, SharedString,
    StyleRefinement, Styled, Toggled, Window, div, prelude::*, px, transparent_black,
};

use super::CheckboxAppearance;

type ChangeCallback = Rc<dyn Fn(bool, &mut Window, &mut App)>;

/// A controlled two- or three-state checkbox with optional label content.
#[derive(IntoElement)]
pub struct Checkbox {
    id: ElementId,
    checked: bool,
    indeterminate: bool,
    disabled: bool,
    label: Option<AnyElement>,
    accessible_label: Option<SharedString>,
    on_change: Option<ChangeCallback>,
    appearance: CheckboxAppearance,
    style: StyleRefinement,
}

impl Checkbox {
    pub fn new(id: impl Into<ElementId>, checked: bool) -> Self {
        Self {
            id: id.into(),
            checked,
            indeterminate: false,
            disabled: false,
            label: None,
            accessible_label: None,
            on_change: None,
            appearance: CheckboxAppearance::default(),
            style: StyleRefinement::default()
                .flex()
                .items_center()
                .gap_2()
                .rounded(px(6.))
                .border_2()
                .border_color(transparent_black()),
        }
    }

    pub fn indeterminate(mut self, indeterminate: bool) -> Self {
        self.indeterminate = indeterminate;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn label(mut self, label: impl IntoElement) -> Self {
        self.label = Some(label.into_any_element());
        self
    }

    pub fn label_text(mut self, label: impl Into<SharedString>) -> Self {
        let label = label.into();
        self.accessible_label = Some(label.clone());
        self.label = Some(div().child(label).into_any_element());
        self
    }

    pub fn aria_label(mut self, label: impl Into<SharedString>) -> Self {
        self.accessible_label = Some(label.into());
        self
    }

    pub fn on_change(mut self, callback: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(callback));
        self
    }

    pub fn appearance(mut self, appearance: CheckboxAppearance) -> Self {
        self.appearance = appearance;
        self
    }
}

impl RenderOnce for Checkbox {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let toggled = if self.indeterminate {
            Toggled::Mixed
        } else {
            self.checked.into()
        };
        let next_value = self.indeterminate || !self.checked;
        let background = if self.indeterminate {
            self.appearance.indeterminate
        } else if self.checked {
            self.appearance.checked
        } else {
            self.appearance.unchecked
        };
        let mark = if self.indeterminate {
            Some("−")
        } else if self.checked {
            Some("✓")
        } else {
            None
        };

        let indicator = div()
            .flex_none()
            .size(self.appearance.indicator_size)
            .rounded(self.appearance.indicator_radius)
            .border_1()
            .border_color(self.appearance.indicator_border)
            .bg(background)
            .flex()
            .items_center()
            .justify_center()
            .text_color(self.appearance.mark)
            .text_xs()
            .children(mark);

        let click_callback = self.on_change.clone();
        let key_callback = self.on_change.clone();
        let action_callback = self.on_change.clone();
        let mut element = div()
            .id(self.id)
            .debug_selector(|| "uic-checkbox".to_string())
            .focusable()
            .tab_stop(!self.disabled)
            .role(Role::CheckBox)
            .aria_toggled(toggled)
            .when_some(self.accessible_label, |this, label| this.aria_label(label))
            .opacity(if self.disabled {
                self.appearance.disabled_opacity
            } else {
                1.0
            })
            .child(indicator)
            .children(self.label.map(|label| div().flex_1().child(label)));
        element.style().refine(&self.style);
        if !self.disabled {
            element = element
                .cursor_pointer()
                .hover(|style| style.border_color(self.appearance.hover_border))
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

impl Styled for Checkbox {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}
