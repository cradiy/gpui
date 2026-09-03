use gpui::{
    App, Bounds, Context, FontWeight, Render, Window, WindowBounds, WindowOptions, div, prelude::*,
    px, rgb, rgba, size,
};
use gpui_effects::{FrostedGlass, FrostedGlassAppearance, TextBlur};
use gpui_platform::application;

struct TextBlurExample;

impl Render for TextBlurExample {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let mut glass = FrostedGlassAppearance::dark();
        glass.tint = rgba(0x11182a80).into();

        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(0x090d18))
            .text_color(rgb(0xe7edf8))
            .child(
                FrostedGlass::with_appearance(glass)
                    .opacity(0.78)
                    .w(px(760.))
                    .rounded(px(28.))
                    .px(px(46.))
                    .py(px(42.))
                    .flex()
                    .flex_col()
                    .gap_5()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgba(0x8be9fdb0))
                            .child("TEXT BLUR · CACHED GLYPH MASKS"),
                    )
                    .child(
                        TextBlur::new("昨日的回声仍悬在风里")
                            .radius(px(5.))
                            .opacity(0.32)
                            .text_size(px(30.))
                            .font_weight(FontWeight::SEMIBOLD),
                    )
                    .child(
                        TextBlur::new("一切失去的流淌都听得见")
                            .radius(px(3.))
                            .opacity(0.48)
                            .text_size(px(30.))
                            .font_weight(FontWeight::SEMIBOLD),
                    )
                    .child(
                        div()
                            .py(px(8.))
                            .text_size(px(34.))
                            .font_weight(FontWeight::BOLD)
                            .child("无人知晓，在这片寂静里。"),
                    )
                    .child(
                        TextBlur::new("我听见遥远的光正在靠近")
                            .radius(px(3.))
                            .opacity(0.48)
                            .text_size(px(30.))
                            .font_weight(FontWeight::SEMIBOLD),
                    )
                    .child(
                        TextBlur::new("下一段旋律正从寂静中醒来")
                            .radius(px(5.))
                            .opacity(0.32)
                            .text_size(px(30.))
                            .font_weight(FontWeight::SEMIBOLD),
                    ),
            )
    }
}

fn main() {
    application().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1100.), px(720.)),
                    cx,
                ))),
                ..Default::default()
            },
            |_, cx| cx.new(|_| TextBlurExample),
        )
        .expect("failed to open text blur example");
    });
}
