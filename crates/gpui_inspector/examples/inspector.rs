use gpui::{
    App, Bounds, Context, Render, Window, WindowBounds, WindowOptions, div, prelude::*, px, rgb,
    size,
};

struct Demo {
    count: usize,
    inspect_initial_card: bool,
}

impl Render for Demo {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.inspect_initial_card {
            self.inspect_initial_card = false;
            window.on_next_frame(|window, cx| {
                let first_card = window
                    .inspector_elements()
                    .iter()
                    .find(|element| element.id.path.global_id.to_string().ends_with("card-0"))
                    .map(|element| element.id.clone());
                if let Some(id) = first_card
                    && let Some(inspector) = window.inspector()
                {
                    inspector.update(cx, |inspector, _| inspector.select(id, window));
                }
            });
        }
        div()
            .id("demo")
            .size_full()
            .p_8()
            .flex()
            .flex_col()
            .gap_6()
            .bg(rgb(0x0f1723))
            .text_color(rgb(0xe2e8f0))
            .child(div().text_2xl().child("Explore your interface"))
            .child(
                div()
                    .text_color(rgb(0x94a3b8))
                    .child("Pick a card or browse the element tree."),
            )
            .child(
                div()
                    .id("toolbar")
                    .flex()
                    .flex_wrap()
                    .gap_3()
                    .child(
                        div()
                            .id("toggle-inspector")
                            .px_4()
                            .py_2()
                            .rounded_md()
                            .bg(rgb(0x334155))
                            .cursor_pointer()
                            .child("Toggle inspector")
                            .on_click(|_, window, cx| window.toggle_inspector(cx)),
                    )
                    .child(
                        div()
                            .id("increment")
                            .px_4()
                            .py_2()
                            .rounded_md()
                            .bg(rgb(0x2563eb))
                            .cursor_pointer()
                            .child(format!("Count: {}", self.count))
                            .on_click(cx.listener(|demo, _, _, cx| {
                                demo.count += 1;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .id("cards")
                    .flex()
                    .flex_col()
                    .gap_4()
                    .children((0..3).map(|index| {
                        div()
                            .id(("card", index))
                            .p_5()
                            .rounded_lg()
                            .bg(rgb([0x243449, 0x28443e, 0x463952][index]))
                            .border_1()
                            .border_color(rgb(0x64748b))
                            .child(format!("Card {}", index + 1))
                            .child(
                                div()
                                    .mt_2()
                                    .text_sm()
                                    .child("Inspect padding, bounds and source location."),
                            )
                    })),
            )
    }
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        gpui_inspector::init(cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1100.), px(760.)),
                    cx,
                ))),
                ..Default::default()
            },
            |window, cx| {
                window.toggle_inspector(cx);
                cx.new(|_| Demo {
                    count: 0,
                    inspect_initial_card: true,
                })
            },
        )
        .unwrap();
        cx.activate(true);
    });
}
