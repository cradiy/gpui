use std::{sync::Arc, time::Duration};

use gpui::{
    Animation, AnimationExt, App, Context, Image, ImageFormat, Render, Window, WindowOptions, div,
    prelude::*, px,
};
use gpui_effects::{album_glow, album_ripples, aurora, color_orbs, plasma};
use gpui_platform::application;

struct EffectsViewer;

impl Render for EffectsViewer {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let colors = [
            gpui::rgb(0xff2d55),
            gpui::rgb(0x5856d6),
            gpui::rgb(0x00d4aa),
            gpui::rgb(0xffcc00),
        ];
        let album_cover = Arc::new(Image::from_bytes(
            ImageFormat::Svg,
            include_bytes!("album-cover.svg").to_vec(),
        ));

        div()
            .size_full()
            .p_6()
            .gap_4()
            .flex()
            .flex_col()
            .bg(gpui::rgb(0x111318))
            .text_color(gpui::white())
            .child(effect_row(
                "Album glow",
                album_glow(album_cover.clone())
                    .size_full()
                    .rounded_xl()
                    .with_animation(
                        "album-glow",
                        Animation::new(Duration::from_secs(12)).repeat(),
                        |effect, time| effect.time(time),
                    ),
            ))
            .child(effect_row(
                "Album ripples",
                album_ripples(album_cover)
                    .size_full()
                    .rounded_xl()
                    .with_animation(
                        "album-ripples",
                        Animation::new(Duration::from_secs(12)).repeat(),
                        |effect, time| effect.time(time),
                    ),
            ))
            .child(effect_row(
                "Aurora",
                aurora(colors).size_full().rounded_xl().with_animation(
                    "aurora",
                    Animation::new(Duration::from_secs(8)).repeat(),
                    |effect, time| effect.time(time),
                ),
            ))
            .child(effect_row(
                "Plasma",
                plasma(colors).size_full().rounded_xl().with_animation(
                    "plasma",
                    Animation::new(Duration::from_secs(7)).repeat(),
                    |effect, time| effect.time(time),
                ),
            ))
            .child(effect_row(
                "Color orbs",
                color_orbs(colors).size_full().rounded_xl().with_animation(
                    "color-orbs",
                    Animation::new(Duration::from_secs(9)).repeat(),
                    |effect, time| effect.time(time),
                ),
            ))
    }
}

fn effect_row(label: &'static str, effect: impl IntoElement) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_4()
        .child(div().w(px(100.)).child(label))
        .child(
            div()
                .flex_1()
                .h(px(150.))
                .rounded_xl()
                .overflow_hidden()
                .child(effect),
        )
}

fn main() {
    application().run(|cx: &mut App| {
        cx.open_window(WindowOptions::default(), |_, cx| cx.new(|_| EffectsViewer))
            .expect("failed to open effects example");
    });
}
