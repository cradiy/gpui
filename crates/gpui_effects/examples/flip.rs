use std::sync::Arc;

use gpui::{App, Context, Image, ImageFormat, Render, Window, WindowOptions, div, prelude::*, px};
use gpui_effects::{Flip, FlipStyle};
use gpui_platform::application;

struct FlipExample {
    book: gpui::Entity<Flip>,
    style: FlipStyle,
}

impl FlipExample {
    fn new(cx: &mut Context<Self>) -> Self {
        let image =
            |bytes: &'static [u8]| Arc::new(Image::from_bytes(ImageFormat::Svg, bytes.to_vec()));
        let previous = image(include_bytes!("flip/previous.svg"));
        let front = image(include_bytes!("flip/front.svg"));
        let back = image(include_bytes!("flip/back.svg"));
        let next = image(include_bytes!("flip/next.svg"));
        Self {
            book: cx.new(|_| Flip::new(previous, front, back, next)),
            style: FlipStyle::Natural,
        }
    }

    fn style_button(
        &self,
        label: &'static str,
        style: FlipStyle,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selected = self.style == style;
        div()
            .id(label)
            .px_4()
            .py_2()
            .rounded_lg()
            .cursor_pointer()
            .bg(if selected {
                gpui::rgb(0x3d526d)
            } else {
                gpui::rgb(0x222832)
            })
            .text_color(if selected {
                gpui::rgb(0xffffff)
            } else {
                gpui::rgb(0xb4bbc5)
            })
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.style = style;
                this.book.update(cx, |book, cx| book.set_style(style, cx));
                cx.notify();
            }))
    }
}

impl Render for FlipExample {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_5()
            .p_8()
            .bg(gpui::rgb(0x101318))
            .text_color(gpui::rgb(0xe8e4dc))
            .child(div().text_2xl().child("Two-page flip"))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(self.style_button("Rigid", FlipStyle::Rigid, cx))
                    .child(self.style_button("Natural", FlipStyle::Natural, cx))
                    .child(self.style_button("Soft · inward", FlipStyle::Soft, cx)),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(gpui::rgb(0x9ba2ad))
                    .child("Press either outer page edge, then drag it across the book."),
            )
            .child(
                div()
                    .w(px(1000.))
                    .h(px(650.))
                    .shadow_xl()
                    .child(self.book.clone()),
            )
    }
}

fn main() {
    application().run(|cx: &mut App| {
        cx.open_window(WindowOptions::default(), |_, cx| cx.new(FlipExample::new))
            .expect("failed to open flip example");
    });
}
