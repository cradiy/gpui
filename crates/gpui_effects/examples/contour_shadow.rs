use std::{cell::Cell, rc::Rc, sync::Arc, time::Instant};

use gpui::{
    App, Bounds, Context, FontWeight, Image, ImageFormat, ImageSource, Pixels, Render, Window,
    WindowBounds, WindowOptions, canvas, div, img, point, prelude::*, px, rgb,
};
use gpui_effects::{ContourReliefOptions, ContourShadowOptions, EffectStage, subtree_effect_chain};
use gpui_platform::application;

const MARK: &[u8] =
    br##"<svg xmlns="http://www.w3.org/2000/svg" width="300" height="90" viewBox="0 0 300 90">
<g fill="none" stroke="#7f98bf" stroke-width="10" stroke-linecap="round" stroke-linejoin="round">
<circle cx="45" cy="45" r="29"/><path d="M125 69 150 19 175 69Z"/>
<path d="m245 12 10 23 24 10-24 10-10 23-10-23-24-10 24-10Z"/></g></svg>"##;

struct ShadowPreview {
    options: ContourShadowOptions,
    target: [f32; 2],
    direction: [f32; 2],
    last_frame: Option<Instant>,
    distance: f32,
    region: Rc<Cell<Bounds<Pixels>>>,
    mark: ImageSource,
    shadow: bool,
    relief: bool,
}

impl ShadowPreview {
    fn new() -> Self {
        Self {
            options: ContourShadowOptions::default(),
            target: [0.58, 0.81],
            direction: [0.58, 0.81],
            last_frame: None,
            distance: 34.,
            region: Rc::new(Cell::new(Bounds::default())),
            mark: Arc::new(Image::from_bytes(ImageFormat::Svg, MARK.to_vec())).into(),
            shadow: true,
            relief: false,
        }
    }

    fn control(
        &self,
        index: usize,
        label: &'static str,
        value: String,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        div()
            .flex_1()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().text_xs().text_color(rgb(0x738099)).child(label))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(value)
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .children([-1., 1.].into_iter().enumerate().map(|(button, delta)| {
                                div()
                                    .id(("adjust", index * 2 + button))
                                    .size(px(30.))
                                    .rounded(px(9.))
                                    .bg(rgb(0xe9eef6))
                                    .hover(|s| s.bg(rgb(0xdce4f1)))
                                    .cursor_pointer()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(if delta < 0. { "−" } else { "+" })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        match index {
                                            0 => {
                                                this.distance =
                                                    (this.distance + delta * 4.).clamp(0., 80.)
                                            }
                                            1 => {
                                                this.options.softness =
                                                    px((f32::from(this.options.softness)
                                                        + delta * 2.)
                                                        .clamp(0., 30.))
                                            }
                                            _ => {
                                                this.options.color.a = (this.options.color.a
                                                    + delta * 0.05)
                                                    .clamp(0., 0.8)
                                            }
                                        }
                                        cx.notify();
                                    }))
                            })),
                    ),
            )
    }

    fn toggle(
        &self,
        index: usize,
        label: &'static str,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id(("toggle", index))
            .px_4()
            .py_2()
            .rounded_full()
            .bg(rgb(0xffffff))
            .hover(|s| s.bg(rgb(0xe4eaf4)))
            .cursor_pointer()
            .text_sm()
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                if index == 0 {
                    this.shadow = !this.shadow;
                } else {
                    this.relief = !this.relief;
                }
                cx.notify();
            }))
    }
}

impl Render for ShadowPreview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(previous) = self.last_frame {
            let now = Instant::now();
            let blend = 1. - (-now.duration_since(previous).as_secs_f32().min(0.05) / 0.16).exp();
            let mut settled = true;
            for (current, target) in self.direction.iter_mut().zip(self.target) {
                *current += (target - *current) * blend;
                settled &= (target - *current).abs() < 0.001;
            }
            if settled {
                self.direction = self.target;
                self.last_frame = None;
            } else {
                self.last_frame = Some(now);
                window.request_animation_frame();
            }
        }
        self.options.offset = point(
            px(self.direction[0] * self.distance),
            px(self.direction[1] * self.distance),
        );
        let mut relief = ContourReliefOptions::default();
        relief.light.direction = [-self.direction[0] * 0.8, -self.direction[1] * 0.8, 0.9];
        relief.light.ambient = 0.55;
        relief.light.intensity = 0.9;
        let source = div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_6()
            .child(
                div()
                    .text_size(px(112.))
                    .line_height(px(132.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(0x8ba3c6))
                    .child("Float"),
            )
            .child(
                div()
                    .text_size(px(32.))
                    .text_color(rgb(0x8694b3))
                    .child("光影之间"),
            )
            .child(img(self.mark.clone()).w(px(300.)).h(px(90.)));
        let surface = subtree_effect_chain(
            source,
            [
                EffectStage::contour_relief(relief).enabled(self.relief),
                EffectStage::contour_shadow(self.options).enabled(self.shadow),
            ],
        )
        .capture_padding(px(112.));
        let region = self.region.clone();
        div()
            .size_full()
            .p_8()
            .bg(rgb(0xf0f3f9))
            .text_color(rgb(0x283a58))
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
                            .child(div().text_size(px(28.)).child("Light & shadow"))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(0x79879f))
                                    .child("Move the pointer to guide the light."),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_3()
                            .child(self.toggle(
                                0,
                                if self.shadow {
                                    "Shadow · On"
                                } else {
                                    "Shadow · Off"
                                },
                                cx,
                            ))
                            .child(self.toggle(
                                1,
                                if self.relief {
                                    "Relief · On"
                                } else {
                                    "Relief · Off"
                                },
                                cx,
                            )),
                    ),
            )
            .child(
                div()
                    .id("surface")
                    .relative()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .rounded(px(28.))
                    .overflow_hidden()
                    .bg(rgb(0xe2e9f3))
                    .child(
                        canvas(move |bounds, _, _| region.set(bounds), |_, _, _, _| {})
                            .absolute()
                            .inset_0()
                            .size_full(),
                    )
                    .child(surface)
                    .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                        let bounds = this.region.get();
                        if bounds.size.width <= px(0.) || bounds.size.height <= px(0.) {
                            return;
                        }
                        let local = event.position - bounds.origin;
                        this.target = [
                            (0.5 - f32::from(local.x) / f32::from(bounds.size.width))
                                .clamp(-0.5, 0.5)
                                * 2.,
                            (0.5 - f32::from(local.y) / f32::from(bounds.size.height))
                                .clamp(-0.5, 0.5)
                                * 2.,
                        ];
                        let length = this.target[0].hypot(this.target[1]).max(1.);
                        this.target = this.target.map(|value| value / length);
                        this.last_frame.get_or_insert_with(Instant::now);
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .p_5()
                    .rounded(px(20.))
                    .bg(rgb(0xffffff))
                    .flex()
                    .gap_8()
                    .child(self.control(0, "DISTANCE", format!("{:.0} px", self.distance), cx))
                    .child(self.control(
                        1,
                        "SOFTNESS",
                        format!("{:.0} px", f32::from(self.options.softness)),
                        cx,
                    ))
                    .child(self.control(
                        2,
                        "OPACITY",
                        format!("{:.0}%", self.options.color.a * 100.),
                        cx,
                    )),
            )
            .when(!window.supports_subtree_effects(), |s| {
                s.child("Subtree effects are unavailable on this renderer.")
            })
    }
}

fn main() {
    application().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    gpui::size(px(1100.), px(820.)),
                    cx,
                ))),
                ..Default::default()
            },
            |_, cx| cx.new(|_| ShadowPreview::new()),
        )
        .expect("failed to open contour shadow example");
    });
}
