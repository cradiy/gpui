use std::{sync::Arc, time::Instant};

use gpui::{
    App, Bounds, Context, Image, ImageFormat, ImageSource, ObjectFit, Render, Window, WindowBounds,
    WindowOptions, div, img, prelude::*, px, rgb, rgba, size,
};
use gpui_effects::{
    SubtreeColorOptions, SubtreeWaveOptions, subtree_blur, subtree_color_adjust, subtree_identity,
    subtree_wave,
};
use gpui_platform::application;

struct SubtreePreview {
    cover: ImageSource,
    mode: usize,
    strength: f32,
    clicks: usize,
    paused: bool,
    elapsed: f32,
    last_frame: Instant,
}

impl SubtreePreview {
    fn new() -> Self {
        Self {
            cover: Arc::new(Image::from_bytes(
                ImageFormat::Svg,
                include_bytes!("album-cover.svg").to_vec(),
            ))
            .into(),
            mode: 2,
            strength: 0.5,
            clicks: 0,
            paused: false,
            elapsed: 0.,
            last_frame: Instant::now(),
        }
    }

    fn card(&self, interactive: bool, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .w_full()
            .rounded(px(24.))
            .overflow_hidden()
            .bg(rgba(0x23263be8))
            .border_1()
            .border_color(rgba(0xffffff18))
            .child(
                img(self.cover.clone())
                    .w_full()
                    .h(px(190.))
                    .object_fit(ObjectFit::Cover),
            )
            .child(
                div()
                    .p_6()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(0x9b9ebb))
                            .child("AFTER HOURS / VOL. 04"),
                    )
                    .child(
                        div()
                            .text_size(px(27.))
                            .text_color(rgb(0xf5f3ff))
                            .child("Midnight garden"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(0xa9aec8))
                            .child("A collection of quiet moments."),
                    )
                    .child(
                        div()
                            .mt_3()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(0xb6bbd4))
                                    .child("12 tracks · 48 min"),
                            )
                            .child(
                                div()
                                    .id(if interactive {
                                        "effect-play"
                                    } else {
                                        "original-play"
                                    })
                                    .px_4()
                                    .py_2()
                                    .rounded_full()
                                    .bg(rgb(0xb9a6ff))
                                    .text_color(rgb(0x1d1930))
                                    .cursor_pointer()
                                    .hover(|s| s.bg(rgb(0xd2c6ff)))
                                    .child(if self.clicks == 0 {
                                        "Play".into()
                                    } else {
                                        format!("Played {}", self.clicks)
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.clicks += 1;
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
    }
}

impl Render for SubtreePreview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
        if self.mode == 2 && self.strength > 0. && !self.paused && window.supports_subtree_effects()
        {
            self.elapsed += now.duration_since(self.last_frame).as_secs_f32();
            window.request_animation_frame();
        }
        self.last_frame = now;
        let card = self.card(true, cx);
        let processed = match self.mode {
            1 => subtree_blur(card, px(self.strength * 8.)),
            2 => subtree_wave(
                card,
                SubtreeWaveOptions {
                    amplitude: px(self.strength * 12.),
                    ..Default::default()
                },
            ),
            3 => subtree_color_adjust(
                card,
                SubtreeColorOptions {
                    saturation: 1. - self.strength,
                    ..Default::default()
                },
            ),
            _ => subtree_identity(card),
        }
        .time(self.elapsed);

        div()
            .size_full()
            .bg(rgb(0x10121d))
            .text_color(rgb(0xf4f3ff))
            .flex()
            .flex_col()
            .p_8()
            .gap_6()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(div().text_size(px(30.)).child("Subtree effects"))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(0x989fb9))
                                    .child("One surface. Every detail."),
                            ),
                    )
                    .child(
                        div().flex().gap_2().children(
                            ["Original", "Blur", "Wave", "Color"]
                                .into_iter()
                                .enumerate()
                                .map(|(index, label)| {
                                    div()
                                        .id(("mode", index))
                                        .px_4()
                                        .py_2()
                                        .rounded_lg()
                                        .bg(if self.mode == index {
                                            rgb(0x5b4b91)
                                        } else {
                                            rgb(0x23263a)
                                        })
                                        .cursor_pointer()
                                        .child(label)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.mode = index;
                                            this.last_frame = Instant::now();
                                            cx.notify();
                                        }))
                                }),
                        ),
                    ),
            )
            .child(
                div()
                    .id("preview-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .justify_center()
                            .gap_8()
                            .py_8()
                            .child(
                                div()
                                    .flex_1()
                                    .max_w(px(340.))
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap_5()
                                    .child(
                                        div().text_sm().text_color(rgb(0x8c94b1)).child("SOURCE"),
                                    )
                                    .child(self.card(false, cx)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .max_w(px(340.))
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap_5()
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(rgb(0xb9a6ff))
                                            .child("COMPOSITED"),
                                    )
                                    .child(processed),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap_4()
                    .child(div().text_sm().text_color(rgb(0x989fb9)).child(format!(
                        "{} · {:.0}%",
                        match self.mode {
                            3 => "Desaturation",
                            _ => "Strength",
                        },
                        self.strength * 100.,
                    )))
                    .children([("less", "−", -0.1), ("more", "+", 0.1)].into_iter().map(
                        |(id, label, delta)| {
                            div()
                                .id(id)
                                .px_4()
                                .py_2()
                                .rounded_lg()
                                .bg(rgb(0x23263a))
                                .cursor_pointer()
                                .child(label)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.strength =
                                        ((this.strength + delta) * 10.).round().clamp(0., 10.)
                                            / 10.;
                                    cx.notify();
                                }))
                        },
                    )),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_sm().text_color(rgb(0x989fb9)).child(
                        if window.supports_subtree_effects() {
                            "Text, artwork and controls share the effect."
                        } else {
                            "Subtree effects are unavailable on this renderer."
                        },
                    ))
                    .child(
                        div()
                            .id("pause")
                            .px_4()
                            .py_2()
                            .rounded_lg()
                            .bg(rgb(0x23263a))
                            .cursor_pointer()
                            .child(if self.paused { "Resume" } else { "Pause" })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.paused = !this.paused;
                                this.last_frame = Instant::now();
                                cx.notify();
                            })),
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
                    size(px(1000.), px(760.)),
                    cx,
                ))),
                ..Default::default()
            },
            |_, cx| cx.new(|_| SubtreePreview::new()),
        )
        .expect("failed to open subtree effects");
    });
}
