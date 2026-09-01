use std::rc::Rc;

use gpui::{
    AnyElement, App, ElementId, IntoElement, RenderOnce, Role, SharedString, StyleRefinement,
    Styled, Window, div, prelude::*, px, transparent_black,
};

use super::RadioGroupAppearance;

type ChangeCallback<T> = Rc<dyn Fn(T, &mut Window, &mut App)>;

struct RadioOption<T> {
    value: T,
    label: AnyElement,
    disabled: bool,
}

/// A controlled radio group with one keyboard focus target and roving selection.
#[derive(IntoElement)]
pub struct RadioGroup<T>
where
    T: Clone + PartialEq + 'static,
{
    id: ElementId,
    selected: T,
    options: Vec<RadioOption<T>>,
    disabled: bool,
    label: Option<SharedString>,
    on_change: Option<ChangeCallback<T>>,
    appearance: RadioGroupAppearance,
    style: StyleRefinement,
}

impl<T> RadioGroup<T>
where
    T: Clone + PartialEq + 'static,
{
    pub fn new(id: impl Into<ElementId>, selected: T) -> Self {
        Self {
            id: id.into(),
            selected,
            options: Vec::new(),
            disabled: false,
            label: None,
            on_change: None,
            appearance: RadioGroupAppearance::default(),
            style: StyleRefinement::default()
                .flex()
                .flex_col()
                .gap_1()
                .rounded(px(6.))
                .border_2()
                .border_color(transparent_black()),
        }
    }

    pub fn option(mut self, value: T, label: impl IntoElement) -> Self {
        self.options.push(RadioOption {
            value,
            label: label.into_any_element(),
            disabled: false,
        });
        self
    }

    pub fn disabled_option(mut self, value: T, label: impl IntoElement) -> Self {
        self.options.push(RadioOption {
            value,
            label: label.into_any_element(),
            disabled: true,
        });
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn on_change(mut self, callback: impl Fn(T, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(callback));
        self
    }

    pub fn appearance(mut self, appearance: RadioGroupAppearance) -> Self {
        self.appearance = appearance;
        self
    }
}

impl<T> RenderOnce for RadioGroup<T>
where
    T: Clone + PartialEq + 'static,
{
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let enabled_values = self
            .options
            .iter()
            .filter(|option| !option.disabled)
            .map(|option| option.value.clone())
            .collect::<Vec<_>>();
        let selected_for_key = self.selected.clone();
        let key_callback = self.on_change.clone();
        let disabled = self.disabled;
        let appearance = self.appearance.clone();
        let options = self.options.into_iter().enumerate().map(|(index, option)| {
            let selected = option.value == self.selected;
            let option_disabled = disabled || option.disabled;
            let callback = self.on_change.clone();
            let value = option.value.clone();
            let indicator = div()
                .flex_none()
                .size(appearance.indicator_size)
                .rounded_full()
                .border_1()
                .border_color(if selected {
                    appearance.selected_border
                } else {
                    appearance.indicator_border
                })
                .bg(appearance.indicator)
                .flex()
                .items_center()
                .justify_center()
                .when(selected, |this| {
                    this.child(
                        div()
                            .size(appearance.dot_size)
                            .rounded_full()
                            .bg(appearance.selected_dot),
                    )
                });
            div()
                .id(("uic-radio-option", index))
                .debug_selector(move || format!("uic-radio-option-{index}"))
                .role(Role::RadioButton)
                .aria_toggled(selected.into())
                .when(selected, |this| this.aria_active_descendant())
                .flex()
                .items_center()
                .gap_2()
                .px_2()
                .py_1()
                .rounded(px(5.))
                .opacity(if option_disabled {
                    appearance.disabled_opacity
                } else {
                    1.0
                })
                .when(!option_disabled, |this| {
                    this.cursor_pointer()
                        .hover(|style| style.border_color(appearance.hover_border))
                        .when_some(callback, |this, callback| {
                            this.on_click(move |_, window, cx| callback(value.clone(), window, cx))
                        })
                })
                .child(indicator)
                .child(div().flex_1().child(option.label))
        });

        let mut element = div()
            .id(self.id)
            .debug_selector(|| "uic-radio-group".to_string())
            .focusable()
            .tab_stop(!disabled)
            .role(Role::RadioGroup)
            .when_some(self.label, |this, label| this.aria_label(label))
            .on_key_down(move |event, window, cx| {
                if disabled {
                    return;
                }
                let direction = match event.keystroke.key.as_str() {
                    "left" | "up" => Some(SelectionDirection::Previous),
                    "right" | "down" => Some(SelectionDirection::Next),
                    "home" => Some(SelectionDirection::First),
                    "end" => Some(SelectionDirection::Last),
                    _ => None,
                };
                if let Some(direction) = direction {
                    if let Some(value) = next_value(&enabled_values, &selected_for_key, direction)
                        && let Some(callback) = key_callback.as_ref()
                    {
                        callback(value.clone(), window, cx);
                    }
                    cx.stop_propagation();
                }
            })
            .children(options);
        element.style().refine(&self.style);
        element.focus_visible(move |style| style.border_color(self.appearance.focus_ring))
    }
}

impl<T> Styled for RadioGroup<T>
where
    T: Clone + PartialEq + 'static,
{
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

#[derive(Clone, Copy)]
enum SelectionDirection {
    Previous,
    Next,
    First,
    Last,
}

fn next_value<'a, T: PartialEq>(
    values: &'a [T],
    selected: &T,
    direction: SelectionDirection,
) -> Option<&'a T> {
    if values.is_empty() {
        return None;
    }
    let selected_index = values.iter().position(|value| value == selected);
    let index = match direction {
        SelectionDirection::First => 0,
        SelectionDirection::Last => values.len() - 1,
        SelectionDirection::Previous => selected_index
            .map(|index| (index + values.len() - 1) % values.len())
            .unwrap_or(values.len() - 1),
        SelectionDirection::Next => selected_index
            .map(|index| (index + 1) % values.len())
            .unwrap_or(0),
    };
    values.get(index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_navigation_wraps_and_recovers_from_a_missing_selection() {
        let values = [10, 20, 30];
        assert_eq!(
            next_value(&values, &30, SelectionDirection::Next),
            Some(&10)
        );
        assert_eq!(
            next_value(&values, &10, SelectionDirection::Previous),
            Some(&30)
        );
        assert_eq!(
            next_value(&values, &99, SelectionDirection::Next),
            Some(&10)
        );
        assert_eq!(
            next_value(&values, &99, SelectionDirection::Previous),
            Some(&30)
        );
    }
}
