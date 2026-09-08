use std::{cell::Cell, rc::Rc, time::Instant};

use gpui::{
    App, Bounds, Context, FontWeight, MouseMoveEvent, Pixels, Point, Render, Window, WindowBounds,
    WindowOptions, canvas, div, point, prelude::*, px, rgb, rgba, size,
};
use gpui_effects::{HolographicOptions, holographic, holographic_masked};
use gpui_platform::application;

struct FoilPreview {
    bounds: Rc<Cell<Bounds<Pixels>>>,
    options: HolographicOptions,
    target_light: Point<f32>,
    last_frame: Instant,
    frozen: bool,
}

impl FoilPreview {
    fn new() -> Self {
        let options = HolographicOptions::default();
        Self {
            bounds: Rc::new(Cell::new(Bounds::default())),
            options,
            target_light: point(options.light.direction[0], options.light.direction[1]),
            last_frame: Instant::now(),
            frozen: false,
        }
    }

    fn track(&mut self, position: Point<Pixels>) {
        let bounds = self.bounds.get();
        if self.frozen || bounds.size.width <= px(0.) || bounds.size.height <= px(0.) {
            return;
        }
        let local = position - bounds.origin;
        let extent = f32::from(bounds.size.width.min(bounds.size.height));
        self.target_light = point(
            (f32::from(local.x - bounds.size.width * 0.5) / extent * 1.3).clamp(-1.4, 1.4),
            (f32::from(local.y - bounds.size.height * 0.5) / extent * 1.3).clamp(-1.4, 1.4),
        );
    }

    fn card(&self) -> impl IntoElement {
        let bounds = self.bounds.clone();
        div()
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .rounded(px(28.))
            .overflow_hidden()
            .child(
                holographic(rgb(0x596373), self.options)
                    .absolute()
                    .inset_0()
                    .size_full(),
            )
            .child(
                canvas(move |region, _, _| bounds.set(region), |_, _, _, _| {})
                    .absolute()
                    .inset_0()
                    .size_full(),
            )
            .child(
                div()
                    .relative()
                    .size_full()
                    .p_8()
                    .flex()
                    .flex_col()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .text_xs()
                            .text_color(rgba(0xffffffb0))
                            .child("PRISM / MATERIAL STUDIES")
                            .child("NO. 001"),
                    )
                    .child(
                        div()
                            .relative()
                            .w_full()
                            .h(px(200.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .children((0..5).map(|i| {
                                let diameter = 88. + i as f32 * 28.;
                                div()
                                    .absolute()
                                    .size(px(diameter))
                                    .rounded_full()
                                    .border_1()
                                    .border_color(rgba(0xffffff45))
                            }))
                            .child(
                                div()
                                    .text_size(px(68.))
                                    .font_weight(FontWeight::LIGHT)
                                    .child("P"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(
                                div()
                                    .text_size(px(48.))
                                    .font_weight(FontWeight::LIGHT)
                                    .child("Chasing light."),
                            )
                            .child(
                                div()
                                    .flex()
                                    .justify_between()
                                    .text_xs()
                                    .text_color(rgba(0xffffffb0))
                                    .child("HOLOGRAPHIC FOIL")
                                    .child("ANGLE / COLOR / REFLECTION"),
                            ),
                    ),
            )
    }

    fn type_sample(&self) -> impl IntoElement {
        div()
            .w(px(300.))
            .h_full()
            .flex()
            .flex_col()
            .justify_between()
            .py_6()
            .pl_6()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_5()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(0x748098))
                            .child("LIGHT IN THE LETTERS"),
                    )
                    .child(holographic_masked(
                        div()
                            .text_size(px(66.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Prism."),
                        rgb(0xa7afbf),
                        self.options,
                    ))
                    .child(holographic_masked(
                        div().text_size(px(36.)).child("光的切面"),
                        rgb(0xa7afbf),
                        self.options,
                    ))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(0x8a95aa))
                            .child("A surface that changes with you."),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_3()
                    .children([0x667580, 0x7b677b, 0x82765d].map(|color| {
                        holographic(rgb(color), self.options)
                            .size(px(64.))
                            .rounded(px(18.))
                    })),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .text_sm()
                    .text_color(rgb(0x8a95aa))
                    .child("Move the pointer to guide the light.")
                    .child("Rest the pointer to hold the reflection."),
            )
    }
}

fn button(
    id: &'static str,
    label: impl Into<gpui::SharedString>,
    active: bool,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .px_4()
        .py_2()
        .rounded_full()
        .cursor_pointer()
        .bg(rgb(if active { 0x33435f } else { 0x1a2537 }))
        .hover(|style| style.bg(rgb(0x36445e)))
        .child(label.into())
}

impl Render for FoilPreview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;
        let light = &mut self.options.light.direction;
        let delta = self.target_light - point(light[0], light[1]);
        if !self.frozen && delta.x.hypot(delta.y) > 0.0005 {
            let step = delta * (1. - (-dt * 14.).exp());
            light[0] += step.x;
            light[1] += step.y;
            window.request_animation_frame();
        }
        div()
            .id("foil-preview")
            .size_full()
            .p_8()
            .bg(rgb(0x0a101c))
            .text_color(rgb(0xeef2fc))
            .flex()
            .flex_col()
            .gap_6()
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if !this.frozen {
                    this.track(event.position);
                    cx.notify();
                }
            }))
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                if !*hovered && !this.frozen {
                    let light = HolographicOptions::default().light.direction;
                    this.target_light = point(light[0], light[1]);
                    cx.notify();
                }
            }))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(div().text_size(px(28.)).child("Holographic"))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(0x8a95aa))
                                    .child("Color lives in the reflection."),
                            ),
                    )
                    .child(
                        button(
                            "freeze",
                            if self.frozen { "Unfreeze" } else { "Freeze" },
                            self.frozen,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.frozen = !this.frozen;
                            this.last_frame = Instant::now();
                            cx.notify();
                        })),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .gap_4()
                    .child(self.card())
                    .child(self.type_sample()),
            )
            .child(
                div()
                    .flex()
                    .gap_3()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(0x748098))
                            .child("SURFACE / DIRECTIONAL LIGHT"),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                button(
                                    "roughness",
                                    format!("Roughness · {:.1}", self.options.surface.roughness),
                                    false,
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.options.surface.roughness =
                                            if this.options.surface.roughness < 0.5 {
                                                0.7
                                            } else {
                                                0.3
                                            };
                                        cx.notify();
                                    },
                                )),
                            )
                            .child(
                                button("texture", "Texture", self.options.surface.texture > 0.)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.options.surface.texture =
                                            if this.options.surface.texture > 0. {
                                                0.
                                            } else {
                                                0.35
                                            };
                                        cx.notify();
                                    })),
                            )
                            .child(
                                button("spectrum", "Spectrum", self.options.iridescence > 0.)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.options.iridescence = if this.options.iridescence > 0.
                                        {
                                            0.
                                        } else {
                                            0.85
                                        };
                                        cx.notify();
                                    })),
                            )
                            .child(button("reset", "Reset", false).on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.options = HolographicOptions::default();
                                    this.target_light = point(
                                        this.options.light.direction[0],
                                        this.options.light.direction[1],
                                    );
                                    this.frozen = false;
                                    this.last_frame = Instant::now();
                                    cx.notify();
                                },
                            ))),
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
                    size(px(1080.), px(760.)),
                    cx,
                ))),
                ..Default::default()
            },
            |_, cx| cx.new(|_| FoilPreview::new()),
        )
        .expect("failed to open holographic example");
    });
}
