use std::{cell::Cell, rc::Rc, sync::Arc, time::Instant};

use gpui::{
    App, Bounds, Context, Image, ImageFormat, ImageSource, MouseButton, MouseDownEvent, ObjectFit,
    Pixels, Point, Render, Window, WindowBounds, WindowOptions, canvas, div, img, point,
    prelude::*, px, rgb, rgba, size,
};
use gpui_effects::{MAX_RIPPLES, Ripple, RippleOptions, subtree_ripples};
use gpui_platform::application;

struct RipplePreview {
    cover: ImageSource,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    waves: Vec<(Point<f32>, Instant)>,
    options: RippleOptions,
}

impl RipplePreview {
    fn new() -> Self {
        Self {
            cover: Arc::new(Image::from_bytes(
                ImageFormat::Svg,
                include_bytes!("album-cover.svg").to_vec(),
            ))
            .into(),
            bounds: Rc::new(Cell::new(Bounds::default())),
            waves: Vec::new(),
            options: RippleOptions::default(),
        }
    }

    fn content(&self) -> impl IntoElement {
        let bounds = self.bounds.clone();
        div()
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(rgb(0x171c2d))
            .child(
                img(self.cover.clone())
                    .absolute()
                    .inset_0()
                    .size_full()
                    .object_fit(ObjectFit::Cover),
            )
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .bg(rgba(0x0c112844))
                    .p_10()
                    .flex()
                    .flex_col()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .text_sm()
                            .text_color(rgb(0xf0eaff))
                            .child("REFRACTION / 01")
                            .child("Image + typography"),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_4()
                            .child(div().text_size(px(80.)).child("Underwater"))
                            .child(
                                div()
                                    .text_size(px(28.))
                                    .text_color(rgb(0xe3e4ff))
                                    .child("光影之间 · Between the waves"),
                            )
                            .child(div().mt_4().flex().gap_3().children(
                                ["Ambient", "Fluid", "After hours"].map(|label| {
                                    div()
                                        .px_4()
                                        .py_2()
                                        .rounded_full()
                                        .bg(rgba(0x121b384d))
                                        .border_1()
                                        .border_color(rgba(0xffffff40))
                                        .text_sm()
                                        .child(label)
                                }),
                            )),
                    )
                    .child(
                        div()
                            .border_t_1()
                            .border_color(rgba(0xffffff60))
                            .pt_4()
                            .flex()
                            .justify_between()
                            .text_sm()
                            .child("Click anywhere to send a ripple")
                            .child("Independent waves · Shared surface"),
                    ),
            )
            .child(
                canvas(move |region, _, _| bounds.set(region), |_, _, _, _| {})
                    .absolute()
                    .inset_0()
                    .size_full(),
            )
    }
}

impl Render for RipplePreview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
        self.waves
            .retain(|(_, start)| now.duration_since(*start) < self.options.duration);
        if !self.waves.is_empty() && window.supports_subtree_effects() {
            window.request_animation_frame();
        }
        let surface = subtree_ripples(
            self.content(),
            self.options,
            self.waves.iter().map(|(center, start)| Ripple {
                center: *center,
                elapsed: now.duration_since(*start),
            }),
        );
        div()
            .size_full()
            .p_8()
            .bg(rgb(0x0b101a))
            .text_color(rgb(0xf4f1ff))
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
                            .child(div().text_size(px(28.)).child("Water ripple"))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(0x8995b1))
                                    .child("A touch sets the surface in motion."),
                            ),
                    )
                    .child(
                        div()
                            .id("clear")
                            .px_4()
                            .py_2()
                            .rounded_full()
                            .bg(rgb(0x23304b))
                            .cursor_pointer()
                            .child("Clear")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.waves.clear();
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .id("water-surface")
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .rounded(px(24.))
                    .overflow_hidden()
                    .cursor_crosshair()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, _, cx| {
                            let bounds = this.bounds.get();
                            if bounds.size.width <= px(0.) || bounds.size.height <= px(0.) {
                                return;
                            }
                            let local = event.position - bounds.origin;
                            let center = point(
                                f32::from(local.x) / f32::from(bounds.size.width),
                                f32::from(local.y) / f32::from(bounds.size.height),
                            );
                            if this.waves.len() == MAX_RIPPLES {
                                this.waves.remove(0);
                            }
                            this.waves.push((center, Instant::now()));
                            cx.notify();
                        }),
                    )
                    .child(surface),
            )
            .child(
                div().flex().gap_4().children(
                    [("Gentle", 6.), ("Natural", 12.), ("Deep", 20.)]
                        .into_iter()
                        .enumerate()
                        .map(|(index, (label, amplitude))| {
                            div()
                                .id(("strength", index))
                                .px_4()
                                .py_2()
                                .rounded_full()
                                .bg(rgb(if self.options.amplitude == px(amplitude) {
                                    0x38486c
                                } else {
                                    0x182133
                                }))
                                .cursor_pointer()
                                .child(label)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.options.amplitude = px(amplitude);
                                    cx.notify();
                                }))
                        }),
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
                    size(px(1080.), px(800.)),
                    cx,
                ))),
                ..Default::default()
            },
            |_, cx| cx.new(|_| RipplePreview::new()),
        )
        .expect("failed to open ripple example");
    });
}
