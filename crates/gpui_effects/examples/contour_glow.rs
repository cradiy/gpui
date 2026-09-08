use std::sync::Arc;

use gpui::{
    App, Bounds, Context, Image, ImageFormat, ImageSource, Render, Window, WindowBounds,
    WindowOptions, div, img, prelude::*, px, rgb, size,
};
use gpui_effects::{ContourGlowOptions, subtree_contour_glow};
use gpui_platform::application;

const MARK: &[u8] =
    br##"<svg xmlns="http://www.w3.org/2000/svg" width="300" height="120" viewBox="0 0 300 120">
<g fill="none" stroke="#b7dbe8" stroke-width="5" stroke-linecap="round" stroke-linejoin="round">
<path d="M48 16 60 43 88 55 60 67 48 96 36 67 8 55 36 43Z"/>
<circle cx="153" cy="56" r="34"/>
<path d="m142 42 27 14-27 14Z"/>
<rect x="221" y="22" width="64" height="68" rx="18"/>
<path d="M236 56h6l6-17 9 34 6-17h7"/>
</g></svg>"##;

struct ContourPreview {
    options: ContourGlowOptions,
    mark: ImageSource,
    enabled: bool,
}

impl ContourPreview {
    fn new() -> Self {
        Self {
            options: ContourGlowOptions::default(),
            mark: Arc::new(Image::from_bytes(ImageFormat::Svg, MARK.to_vec())).into(),
            enabled: true,
        }
    }

    fn content(&self) -> impl IntoElement {
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap_10()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .text_size(px(60.))
                            .text_color(rgb(0xb7dbe8))
                            .child("Outline"),
                    )
                    .child(
                        div()
                            .text_size(px(28.))
                            .text_color(rgb(0xb7dbe8))
                            .child("光，沿着轮廓"),
                    ),
            )
            .child(img(self.mark.clone()).w(px(300.)).h(px(120.)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_6()
                    .child(
                        div()
                            .text_size(px(42.))
                            .text_color(rgb(0xb7dbe8))
                            .child("Aa 08"),
                    )
                    .child(
                        div()
                            .text_size(px(18.))
                            .text_color(rgb(0xb7dbe8))
                            .child("Soft edges.\nOpen spaces."),
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
            .flex()
            .flex_col()
            .gap_3()
            .child(div().text_xs().text_color(rgb(0x7f91ad)).child(label))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .child(value)
                    .child(div().flex().gap_1().children(
                        [(-1., "−"), (1., "+")].into_iter().enumerate().map(
                            |(button, (delta, label))| {
                                div()
                                    .id(("adjust", index * 2 + button))
                                    .size(px(30.))
                                    .rounded(px(9.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .bg(rgb(0x24314a))
                                    .cursor_pointer()
                                    .hover(|style| style.bg(rgb(0x324564)))
                                    .child(label)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        match index {
                                            0 => {
                                                this.options.radius =
                                                    px((f32::from(this.options.radius)
                                                        + delta * 4.)
                                                        .clamp(4., 40.))
                                            }
                                            1 => {
                                                this.options.edge_width =
                                                    px((f32::from(this.options.edge_width)
                                                        + delta * 0.5)
                                                        .clamp(0., 6.))
                                            }
                                            _ => {
                                                this.options.intensity = (this.options.intensity
                                                    + delta * 0.2)
                                                    .clamp(0., 4.)
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

impl Render for ContourPreview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(0x090f1a))
            .text_color(rgb(0xe6efff))
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
                            .child(div().text_size(px(28.)).child("Contour light"))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(0x8395b0))
                                    .child("Light follows the shape."),
                            ),
                    )
                    .child(
                        div()
                            .id("toggle")
                            .px_4()
                            .py_2()
                            .rounded_full()
                            .bg(rgb(0x22324c))
                            .cursor_pointer()
                            .child(if self.enabled {
                                "Glow · On"
                            } else {
                                "Glow · Off"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.enabled = !this.enabled;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .id("preview")
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .flex()
                            .justify_center()
                            .gap(px(80.))
                            .p(px(48.))
                            .children([false, true].map(|processed| {
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .max_w(px(420.))
                                    .flex()
                                    .flex_col()
                                    .gap_8()
                                    .child(
                                        div().text_xs().text_color(rgb(0x8395b0)).child(
                                            if processed { "CONTOUR LIGHT" } else { "SOURCE" },
                                        ),
                                    )
                                    .child(
                                        subtree_contour_glow(self.content(), self.options)
                                            .enabled(processed && self.enabled),
                                    )
                            })),
                    ),
            )
            .child(
                div()
                    .rounded(px(20.))
                    .bg(rgb(0x111b2b))
                    .p_5()
                    .flex()
                    .gap_8()
                    .child(self.control(
                        0,
                        "RADIUS",
                        format!("{:.0} px", f32::from(self.options.radius)),
                        cx,
                    ))
                    .child(self.control(
                        1,
                        "EDGE WIDTH",
                        format!("{:.1} px", f32::from(self.options.edge_width)),
                        cx,
                    ))
                    .child(self.control(
                        2,
                        "INTENSITY",
                        format!("{:.1}×", self.options.intensity),
                        cx,
                    )),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div().flex().gap_3().children(
                            [("Ice", 0x76deff), ("Violet", 0xbda0ff), ("Amber", 0xffc079)]
                                .into_iter()
                                .enumerate()
                                .map(|(index, (label, color))| {
                                    div()
                                        .id(("color", index))
                                        .px_4()
                                        .py_2()
                                        .rounded_full()
                                        .cursor_pointer()
                                        .bg(rgb(if self.options.color == rgb(color) {
                                            0x2c3b55
                                        } else {
                                            0x172238
                                        }))
                                        .text_color(rgb(color))
                                        .child(label)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.options.color = rgb(color);
                                            cx.notify();
                                        }))
                                }),
                        ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(0x8395b0))
                            .child("Alpha contour · Distance field"),
                    ),
            )
            .when(!window.supports_subtree_effects(), |root| {
                root.child("Subtree effects are unavailable on this renderer.")
            })
    }
}

fn main() {
    application().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1100.), px(820.)),
                    cx,
                ))),
                ..Default::default()
            },
            |_, cx| cx.new(|_| ContourPreview::new()),
        )
        .expect("failed to open contour light example");
    });
}
