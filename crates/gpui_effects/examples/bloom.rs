use std::sync::Arc;

use gpui::{
    App, Bounds, Context, Image, ImageFormat, ImageSource, ObjectFit, Render, Window, WindowBounds,
    WindowOptions, div, img, prelude::*, px, rgb, size,
};
use gpui_effects::{BloomOptions, subtree_bloom};
use gpui_platform::application;

struct BloomPreview {
    cover: ImageSource,
    options: BloomOptions,
    enabled: bool,
}

impl BloomPreview {
    fn new() -> Self {
        Self {
            cover: Arc::new(Image::from_bytes(
                ImageFormat::Svg,
                include_bytes!("album-cover.svg").to_vec(),
            ))
            .into(),
            options: BloomOptions::default(),
            enabled: true,
        }
    }

    fn content(&self) -> impl IntoElement {
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap_8()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(48.))
                            .text_color(rgb(0x9aeeff))
                            .child("Luminous"),
                    )
                    .child(
                        div()
                            .text_size(px(24.))
                            .text_color(rgb(0xc7b4ff))
                            .child("微光 · After hours"),
                    )
                    .child(
                        div()
                            .mt_2()
                            .text_sm()
                            .text_color(rgb(0x515a70))
                            .child("Keep the quiet details."),
                    ),
            )
            .child(
                img(self.cover.clone())
                    .w_full()
                    .h(px(210.))
                    .rounded(px(18.))
                    .object_fit(ObjectFit::Cover),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_4()
                    .children(
                        [0x8fecff, 0xc3a7ff, 0xffcb8c]
                            .map(|color| div().size(px(12.)).rounded_full().bg(rgb(color))),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(0xc9d1e7))
                            .child("Light in color"),
                    ),
            )
    }

    fn control(
        &self,
        index: usize,
        label: &'static str,
        value: String,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().text_xs().text_color(rgb(0x7f8ba5)).child(label))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(div().text_sm().child(value))
                    .child(div().flex().gap_1().children(
                        [(-1., "−"), (1., "+")].into_iter().enumerate().map(
                            |(button, (delta, label))| {
                                div()
                                    .id(("adjust", index * 2 + button))
                                    .size(px(28.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(7.))
                                    .bg(rgb(0x1b2334))
                                    .hover(|s| s.bg(rgb(0x2b3750)))
                                    .cursor_pointer()
                                    .child(label)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        match index {
                                            0 => {
                                                this.options.threshold = (this.options.threshold
                                                    + delta * 0.05)
                                                    .clamp(0., 1.)
                                            }
                                            1 => {
                                                this.options.intensity = (this.options.intensity
                                                    + delta * 0.2)
                                                    .clamp(0., 3.)
                                            }
                                            2 => {
                                                this.options.radius =
                                                    px((f32::from(this.options.radius)
                                                        + delta * 4.)
                                                        .clamp(0., 96.))
                                            }
                                            _ => {
                                                this.options.soft_knee = (this.options.soft_knee
                                                    + delta * 0.05)
                                                    .clamp(0., 1.)
                                            }
                                        }
                                        cx.notify();
                                    }))
                            },
                        ),
                    )),
            )
    }
}

impl Render for BloomPreview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(0x0b101a))
            .text_color(rgb(0xe9efff))
            .p_8()
            .flex()
            .flex_col()
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
                            .child(div().text_size(px(28.)).child("Bloom"))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(0x7f8ba5))
                                    .child("Soft light. Sharp detail."),
                            ),
                    )
                    .child(
                        div()
                            .id("toggle")
                            .px_4()
                            .py_2()
                            .rounded_full()
                            .bg(if self.enabled {
                                rgb(0x283858)
                            } else {
                                rgb(0x1b2334)
                            })
                            .cursor_pointer()
                            .child(if self.enabled {
                                "Bloom · On"
                            } else {
                                "Bloom · Off"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.enabled = !this.enabled;
                                cx.notify();
                            })),
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
                            .flex()
                            .justify_center()
                            .gap(px(80.))
                            .p(px(64.))
                            .children([false, true].map(|processed| {
                                div()
                                    .flex_1()
                                    .max_w(px(440.))
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap_8()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(0x7f8ba5))
                                            .child(if processed { "BLOOM" } else { "SOURCE" }),
                                    )
                                    .child(
                                        subtree_bloom(self.content(), self.options)
                                            .enabled(processed && self.enabled),
                                    )
                            })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_6()
                    .p_5()
                    .rounded(px(16.))
                    .bg(rgb(0x111927))
                    .child(self.control(
                        0,
                        "THRESHOLD",
                        format!("{:.2}", self.options.threshold),
                        cx,
                    ))
                    .child(self.control(
                        1,
                        "INTENSITY",
                        format!("{:.1}×", self.options.intensity),
                        cx,
                    ))
                    .child(self.control(
                        2,
                        "RADIUS",
                        format!("{:.0} px", f32::from(self.options.radius)),
                        cx,
                    ))
                    .child(self.control(
                        3,
                        "SOFT KNEE",
                        format!("{:.2}", self.options.soft_knee),
                        cx,
                    )),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_xs().text_color(rgb(0x7f8ba5)).child(
                        if window.supports_subtree_effects() {
                            "Highlight extraction · Separable blur · Composite"
                        } else {
                            "Bloom is unavailable on this renderer."
                        },
                    ))
                    .child(
                        div()
                            .id("resolution")
                            .px_3()
                            .py_2()
                            .rounded(px(8.))
                            .bg(rgb(0x1b2334))
                            .text_sm()
                            .cursor_pointer()
                            .child(format!("Resolution · 1/{}", self.options.downsample))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.options.downsample = match this.options.downsample {
                                    1 => 2,
                                    2 => 4,
                                    _ => 1,
                                };
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
                    size(px(1060.), px(800.)),
                    cx,
                ))),
                ..Default::default()
            },
            |_, cx| cx.new(|_| BloomPreview::new()),
        )
        .expect("failed to open bloom example");
    });
}
