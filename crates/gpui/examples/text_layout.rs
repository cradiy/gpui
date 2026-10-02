#![cfg_attr(target_family = "wasm", no_main)]

use gpui::{
    App, Bounds, Context, FontStyle, FontWeight, InteractiveText, StyledText, Window, WindowBounds,
    WindowOptions, div, prelude::*, px, size,
};
use gpui_platform::application;

struct HelloWorld {}

impl Render for HelloWorld {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let selectable = "Selectable text\nDrag across these styled lines, then copy with Ctrl/Cmd+C. 中文与 emoji 😀 remain intact. Click here to open the GPUI repository.";
        let link_start = selectable.find("Click here").unwrap();
        div()
            .bg(gpui::white())
            .flex()
            .flex_col()
            .gap_2()
            .p_4()
            .size_full()
            .child(
                div()
                    .w(px(420.))
                    .p_3()
                    .border_1()
                    .border_color(gpui::rgb(0xd4dbe5))
                    .child(
                        InteractiveText::new(
                            "selectable-example",
                            StyledText::new(selectable)
                                .with_highlights([(0..15, FontWeight::BOLD.into())]),
                        )
                        .selectable(gpui::rgba(0x4488ff55))
                        .on_click(
                            vec![link_start..link_start + "Click here".len()],
                            |_, _, cx| cx.open_url("https://github.com/cradiy/gpui-fusion"),
                        ),
                    ),
            )
            .child(div().child("Text left"))
            .child(div().text_center().child("Text center"))
            .child(div().text_right().child("Text right"))
            .child(div().text_decoration_1().child("Text left (underline)"))
            .child(
                div()
                    .text_center()
                    .text_decoration_1()
                    .child("Text center (underline)"),
            )
            .child(
                div()
                    .text_right()
                    .text_decoration_1()
                    .child("Text right (underline)"),
            )
            .child(div().line_through().child("Text left (line_through)"))
            .child(
                div()
                    .text_center()
                    .line_through()
                    .child("Text center (line_through)"),
            )
            .child(
                div()
                    .text_right()
                    .line_through()
                    .child("Text right (line_through)"),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .justify_between()
                    .child(
                        div()
                            .w(px(400.))
                            .border_1()
                            .border_color(gpui::blue())
                            .p_1()
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_center()
                            .child("A long non-wrapping text align center"),
                    )
                    .child(
                        div()
                            .w_32()
                            .border_1()
                            .border_color(gpui::blue())
                            .p_1()
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_right()
                            .child("100%"),
                    ),
            )
            .child(div().flex().gap_2().justify_between().child(
                StyledText::new("ABCD").with_highlights([
                    (0..1, FontWeight::EXTRA_BOLD.into()),
                    (2..3, FontStyle::Italic.into()),
                ]),
            ))
    }
}

fn run_example() {
    application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(800.0), px(600.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|_| HelloWorld {}),
        )
        .unwrap();
        cx.activate(true);
    });
}

#[cfg(not(target_family = "wasm"))]
fn main() {
    run_example();
}

#[cfg(target_family = "wasm")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    gpui_platform::web_init();
    run_example();
}
