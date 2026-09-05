use gpui::{
    Bounds, Context, Entity, Render, Window, WindowBounds, WindowOptions, div, prelude::*, px, rgb,
    size,
};
use uic::components::input::{Input, TextInput};

struct InputExample {
    title: Entity<TextInput>,
    notes: Entity<TextInput>,
    draft: Entity<TextInput>,
}

impl InputExample {
    fn new(cx: &mut Context<Self>) -> Self {
        let notes = (1..=24)
            .map(|line| {
                format!(
                    "Line {line:02}: Edit this text, scroll with the mouse wheel, or drag the scrollbar to explore the document."
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        Self {
            title: cx.new(|cx| TextInput::new(cx).placeholder("Give your notes a title")),
            notes: cx.new(|cx| TextInput::new(cx).multiline().initial_value(notes)),
            draft: cx.new(|cx| {
                TextInput::new(cx)
                    .multiline()
                    .placeholder("Start typing here. Add more than three rows to reveal the thumb.")
            }),
        }
    }
}

impl Render for InputExample {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .p_6()
            .bg(rgb(0xf4f6f8))
            .text_color(rgb(0x172033))
            .child(
                div()
                    .w_full()
                    .max_w(px(640.))
                    .p_8()
                    .rounded(px(20.))
                    .bg(rgb(0xffffff))
                    .shadow_lg()
                    .flex()
                    .flex_col()
                    .gap_5()
                    .child(div().text_2xl().child("Input and scrollbar"))
                    .child(field(
                        "Single line",
                        Input::new(&self.title).text_color(rgb(0x000000)),
                    ))
                    .child(field(
                        "Scrollable notes · 6 visible rows",
                        Input::new(&self.notes)
                            .rows(6)
                            .text_color(rgb(0x172033))
                            .pr(px(18.))
                            .scrollbar(|bar| {
                                bar
                                    .w(px(12.))
                                    .right(px(2.))
                                    .top(px(6.))
                                    .bottom(px(6.))
                                    .auto_hide(false)
                            }),
                    ))
                    .child(field(
                        "Empty draft · 3 visible rows",
                        Input::new(&self.draft).rows(3).text_color(rgb(0x172033)),
                    ))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(0x64748b))
                            .child("Multiline fields wrap automatically. Enter inserts a newline; the caret scrolls into view as you type."),
                    ),
            )
    }
}

fn field(label: &'static str, input: Input) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(div().text_sm().child(label))
        .child(input)
}

fn main() {
    gpui_platform::application().run(|cx| {
        uic::init(cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(780.), px(740.)),
                    cx,
                ))),
                ..Default::default()
            },
            |_, cx| cx.new(InputExample::new),
        )
        .expect("failed to open input example window");
        cx.activate(true);
    });
}
