use std::{
    cell::Cell,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::{
    App, Bounds, Context, Image, ImageFormat, ImageSource, MouseButton, ObjectFit, Pixels, Point,
    Render, Window, WindowBounds, WindowOptions, canvas, div, img, point, prelude::*, px, rgb,
    size,
};
use gpui_effects::{MotionBlurOptions, subtree_motion_blur};
use gpui_platform::application;

struct MotionBlurPreview {
    cover: ImageSource,
    regions: [Rc<Cell<Bounds<Pixels>>>; 2],
    offset: Point<Pixels>,
    velocity: Point<Pixels>,
    drag: Option<(Point<Pixels>, Point<Pixels>)>,
    last_pointer: Instant,
    last_frame: Instant,
    options: MotionBlurOptions,
}

impl MotionBlurPreview {
    fn new() -> Self {
        Self {
            cover: Arc::new(Image::from_bytes(
                ImageFormat::Svg,
                include_bytes!("album-cover.svg").to_vec(),
            ))
            .into(),
            regions: std::array::from_fn(|_| Rc::new(Cell::new(Bounds::default()))),
            offset: point(px(0.), px(0.)),
            velocity: point(px(0.), px(0.)),
            drag: None,
            last_pointer: Instant::now(),
            last_frame: Instant::now(),
            options: MotionBlurOptions::default(),
        }
    }

    fn advance(&mut self) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32().min(0.05);
        self.last_frame = now;
        if self.drag.is_some() {
            if now.duration_since(self.last_pointer).as_secs_f32() > 0.04 {
                self.velocity = self.velocity * (-dt * 35.).exp();
                if self.speed() < 1. {
                    self.velocity = point(px(0.), px(0.));
                }
            }
        } else {
            let steps = (dt * 240.).ceil().max(1.) as usize;
            let step = dt / steps as f32;
            for _ in 0..steps {
                self.velocity = self.velocity - (self.offset * 150. + self.velocity * 16.) * step;
                self.offset = self.offset + self.velocity * step;
            }
            if f32::from(self.offset.x).hypot(f32::from(self.offset.y)) < 0.1 && self.speed() < 1. {
                self.offset = point(px(0.), px(0.));
                self.velocity = point(px(0.), px(0.));
            }
        }
    }

    fn speed(&self) -> f32 {
        f32::from(self.velocity.x).hypot(f32::from(self.velocity.y))
    }

    fn release(&mut self) {
        self.drag = None;
        self.last_frame = Instant::now();
    }

    fn panel(&self, index: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let region = self.regions[index].clone();
        let content = div()
            .id(("card", index))
            .w(px(220.))
            .h(px(270.))
            .rounded(px(22.))
            .overflow_hidden()
            .bg(rgb(0x202b41))
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                    this.advance();
                    this.drag = Some((event.position, this.offset));
                    this.velocity = point(px(0.), px(0.));
                    this.last_pointer = Instant::now();
                    cx.notify();
                }),
            )
            .child(
                img(self.cover.clone())
                    .w_full()
                    .h(px(150.))
                    .object_fit(ObjectFit::Cover),
            )
            .child(
                div()
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(0x9baced))
                            .child("AFTER HOURS / 04"),
                    )
                    .child(div().text_size(px(24.)).child("Night drive"))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(0xa6b5cf))
                            .child("夜行 · 12 tracks"),
                    ),
            );
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(if index == 0 { 0x8699b8 } else { 0x8ee5ee }))
                    .child(if index == 0 { "SOURCE" } else { "MOTION BLUR" }),
            )
            .child(
                div()
                    .relative()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .rounded(px(24.))
                    .bg(rgb(0x111b2a))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        canvas(move |bounds, _, _| region.set(bounds), |_, _, _, _| {})
                            .absolute()
                            .inset_0()
                            .size_full(),
                    )
                    .child(
                        div()
                            .relative()
                            .left(self.offset.x)
                            .top(self.offset.y)
                            .w(px(220.))
                            .h(px(270.))
                            .flex_shrink_0()
                            .child(subtree_motion_blur(
                                content,
                                MotionBlurOptions {
                                    velocity: if index == 0 {
                                        point(px(0.), px(0.))
                                    } else {
                                        self.velocity
                                    },
                                    ..self.options
                                },
                            )),
                    ),
            )
    }
}

impl Render for MotionBlurPreview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.advance();
        if !window.is_window_active() {
            self.release();
        }
        if window.is_window_active()
            && (self.speed() > 0. || self.offset != point(px(0.), px(0.)) && self.drag.is_none())
        {
            window.request_animation_frame();
        }
        div()
            .id("motion-blur-preview")
            .size_full()
            .p_6()
            .bg(rgb(0x0b111d))
            .text_color(rgb(0xeaf1ff))
            .flex()
            .flex_col()
            .gap_6()
            .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                if let Some((origin, base)) = this.drag {
                    if event.pressed_button != Some(MouseButton::Left) {
                        this.release();
                    } else {
                        let now = Instant::now();
                        let dt = now
                            .duration_since(this.last_pointer)
                            .as_secs_f32()
                            .max(0.001);
                        let width = this
                            .regions
                            .iter()
                            .map(|r| f32::from(r.get().size.width))
                            .fold(f32::INFINITY, f32::min);
                        let height = this
                            .regions
                            .iter()
                            .map(|r| f32::from(r.get().size.height))
                            .fold(f32::INFINITY, f32::min);
                        let limit_x = (width * 0.5 - 130.).max(0.);
                        let limit_y = (height * 0.5 - 155.).max(0.);
                        let desired = base + event.position - origin;
                        let offset = point(
                            px(f32::from(desired.x).clamp(-limit_x, limit_x)),
                            px(f32::from(desired.y).clamp(-limit_y, limit_y)),
                        );
                        let measured = (offset - this.offset) * (1. / dt);
                        let blend = 1. - (-dt * 45.).exp();
                        this.velocity = this.velocity * (1. - blend) + measured * blend;
                        this.offset = offset;
                        this.last_pointer = now;
                    }
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.release();
                    cx.notify();
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.release();
                    cx.notify();
                }),
            )
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                if !hovered && this.drag.is_some() {
                    this.release();
                    cx.notify();
                }
            }))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().text_size(px(30.)).child("In motion"))
                    .child(
                        div()
                            .text_color(rgb(0x94a8c5))
                            .child("Drag either card · Release to return"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_6()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .child(self.panel(0, cx))
                    .child(self.panel(1, cx)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .child(
                        div().flex().gap_2().children(
                            [
                                ("Subtle", 120., 24.),
                                ("Balanced", 60., 40.),
                                ("Strong", 30., 64.),
                            ]
                            .into_iter()
                            .enumerate()
                            .map(
                                |(index, (label, shutter, distance))| {
                                    let exposure = Duration::from_secs_f64(1. / shutter);
                                    div()
                                        .id(("exposure", index))
                                        .px_4()
                                        .py_2()
                                        .rounded_full()
                                        .cursor_pointer()
                                        .bg(rgb(if self.options.exposure == exposure {
                                            0x365270
                                        } else {
                                            0x1c293d
                                        }))
                                        .child(label)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.options.exposure = exposure;
                                            this.options.max_distance = px(distance);
                                            cx.notify();
                                        }))
                                },
                            ),
                        ),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(0x94a8c5))
                            .child(format!("{:.0} px/s", self.speed())),
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
                    size(px(1100.), px(760.)),
                    cx,
                ))),
                ..Default::default()
            },
            |_, cx| cx.new(|_| MotionBlurPreview::new()),
        )
        .expect("failed to open motion blur example");
    });
}
