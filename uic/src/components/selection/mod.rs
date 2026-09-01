mod appearance;
mod checkbox;
mod radio_group;
mod switch;

pub use appearance::{CheckboxAppearance, RadioGroupAppearance, SwitchAppearance};
pub use checkbox::Checkbox;
pub use radio_group::RadioGroup;
pub use switch::Switch;

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use gpui::{
        Context, IntoElement, Modifiers, Render, TestAppContext, VisualTestContext, Window, div,
        prelude::*, px, size,
    };

    use super::*;

    struct TestSelection {
        checkbox_change: Rc<Cell<Option<bool>>>,
        switch_change: Rc<Cell<Option<bool>>>,
        radio_change: Rc<Cell<Option<u8>>>,
    }

    impl Render for TestSelection {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let checkbox_change = self.checkbox_change.clone();
            let switch_change = self.switch_change.clone();
            let radio_change = self.radio_change.clone();
            div()
                .p(px(40.))
                .flex()
                .flex_col()
                .gap_4()
                .child(
                    Checkbox::new("test-checkbox", false)
                        .label_text("Checkbox")
                        .on_change(move |value, _, _| checkbox_change.set(Some(value))),
                )
                .child(
                    Switch::new("test-switch", false)
                        .label("Switch")
                        .on_change(move |value, _, _| switch_change.set(Some(value))),
                )
                .child(
                    RadioGroup::new("test-radio", 1_u8)
                        .option(1, "One")
                        .option(2, "Two")
                        .on_change(move |value, _, _| radio_change.set(Some(value))),
                )
        }
    }

    #[gpui::test]
    fn pointer_activation_emits_controlled_values(cx: &mut TestAppContext) {
        let window = cx.open_window(size(px(320.), px(260.)), |_, _| TestSelection {
            checkbox_change: Rc::new(Cell::new(None)),
            switch_change: Rc::new(Cell::new(None)),
            radio_change: Rc::new(Cell::new(None)),
        });
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.update(|window, cx| {
            window.draw(cx).clear();
        });

        let checkbox = visual.debug_bounds("uic-checkbox").unwrap();
        let switch = visual.debug_bounds("uic-switch").unwrap();
        let second_radio = visual.debug_bounds("uic-radio-option-1").unwrap();
        visual.simulate_click(checkbox.center(), Modifiers::default());
        visual.simulate_click(switch.center(), Modifiers::default());
        visual.simulate_click(second_radio.center(), Modifiers::default());

        window
            .update(&mut visual.cx, |view, _, _| {
                assert_eq!(view.checkbox_change.get(), Some(true));
                assert_eq!(view.switch_change.get(), Some(true));
                assert_eq!(view.radio_change.get(), Some(2));
            })
            .unwrap();
    }
}
