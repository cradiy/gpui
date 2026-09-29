use super::{DropdownState, dropdown};
use gpui::{
    AppContext, Context, Entity, FocusHandle, MouseButton, Render, TestAppContext, Window, div,
    point, prelude::*, px,
};

struct TestDropdown {
    state: Entity<DropdownState>,
    outside_focus: FocusHandle,
    outside_clicks: usize,
    menu_clicks: usize,
}

impl Render for TestDropdown {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .child(
                div()
                    .absolute()
                    .left(px(20.))
                    .top(px(20.))
                    .w(px(100.))
                    .child(
                        dropdown(&self.state)
                            .w(px(160.))
                            .h(px(80.))
                            .trigger(div().w(px(100.)).h(px(30.)))
                            .menu(div().size_full().on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, _| this.menu_clicks += 1),
                            )),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .left(px(300.))
                    .top(px(20.))
                    .w(px(80.))
                    .h(px(30.))
                    .occlude()
                    .track_focus(&self.outside_focus)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            this.outside_clicks += 1;
                            this.outside_focus.focus(window, cx);
                        }),
                    ),
            )
    }
}

#[gpui::test]
fn outside_click_closes_without_blur_and_preserves_trigger_and_menu_clicks(
    cx: &mut TestAppContext,
) {
    let (view, cx) = cx.add_window_view(|window, cx| TestDropdown {
        state: cx.new(|cx| DropdownState::new(window, cx)),
        outside_focus: cx.focus_handle(),
        outside_clicks: 0,
        menu_clicks: 0,
    });
    let state = cx.update(|_, cx| view.read(cx).state.clone());
    let trigger = point(px(50.), px(35.));
    let blank = point(px(400.), px(300.));
    let inside_menu = point(px(50.), px(80.));
    let outside_control = point(px(330.), px(35.));
    let open = |cx: &mut gpui::VisualTestContext| cx.update(|_, cx| state.read(cx).is_open());
    let click = |position, cx: &mut gpui::VisualTestContext| {
        cx.simulate_click(position, gpui::Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear());
    };

    click(trigger, cx);
    assert!(open(cx));
    click(inside_menu, cx);
    assert!(open(cx));
    assert_eq!(cx.update(|_, cx| view.read(cx).menu_clicks), 1);
    click(trigger, cx);
    assert!(!open(cx), "trigger must close without reopening");

    click(trigger, cx);
    assert!(open(cx));
    click(blank, cx);
    assert!(!open(cx), "blank areas must close without changing focus");

    click(trigger, cx);
    click(outside_control, cx);
    assert!(!open(cx));
    cx.update(|window, cx| {
        let view = view.read(cx);
        assert_eq!(view.outside_clicks, 1);
        assert!(view.outside_focus.is_focused(window));
    });

    click(trigger, cx);
    cx.simulate_keystrokes("escape");
    assert!(!open(cx));

    click(trigger, cx);
    cx.simulate_mouse_down(blank, MouseButton::Right, gpui::Modifiers::default());
    assert!(!open(cx), "right clicks outside must also dismiss");
}
