use std::{cell::Cell, rc::Rc, time::Duration};

use gpui::{
    App, Bounds, Context, MouseButton, MouseMoveEvent, PathBuilder, Pixels, Point, Render, Window,
    WindowBounds, WindowOptions, canvas, div, point, prelude::*, px, quad, rgb, rgba, size,
};
use gpui_effects::{BloomOptions, EffectStage, Feedback, FeedbackOptions, subtree_effect_chain};
use gpui_platform::application;

const COLORS: [u32; 3] = [0x82e9ff, 0xc0a5ff, 0xffc38b];

struct StrokeSegment {
    start: Point<Pixels>,
    control: Point<Pixels>,
    end: Point<Pixels>,
}

struct FeedbackPreview {
    feedback: Feedback,
    surface_bounds: Rc<Cell<Bounds<Pixels>>>,
    segments: Vec<StrokeSegment>,
    caps: Vec<Point<Pixels>>,
    last_point: Option<Point<Pixels>>,
    stroke_end: Option<Point<Pixels>>,
    dragging: bool,
    has_drawn: bool,
    color: usize,
    bloom: bool,
}

impl FeedbackPreview {
    fn new() -> Self {
        Self {
            feedback: Feedback::new(FeedbackOptions {
                downsample: 1,
                ..Default::default()
            }),
            surface_bounds: Rc::new(Cell::new(Bounds::default())),
            segments: Vec::new(),
            caps: Vec::new(),
            last_point: None,
            stroke_end: None,
            dragging: false,
            has_drawn: false,
            color: 0,
            bloom: true,
        }
    }

    fn record(&mut self, position: Point<Pixels>) {
        if self.feedback.is_paused() {
            return;
        }
        let local = position - self.surface_bounds.get().origin;
        if let Some(previous) = self.last_point {
            let delta = local - previous;
            if f32::from(delta.x).powi(2) + f32::from(delta.y).powi(2) < 0.25 {
                return;
            }
            let end = (previous + local) * 0.5;
            self.segments.push(StrokeSegment {
                start: self.stroke_end.unwrap_or(previous),
                control: previous,
                end,
            });
            self.stroke_end = Some(end);
        } else {
            self.caps.push(local);
            self.stroke_end = Some(local);
        }
        self.last_point = Some(local);
        self.has_drawn = true;
        self.feedback.emit();
    }

    fn finish_stroke(&mut self) {
        if let Some(end) = self.last_point.take() {
            if let Some(start) = self.stroke_end.take()
                && start != end
            {
                self.segments.push(StrokeSegment {
                    start,
                    control: end,
                    end,
                });
                self.caps.push(end);
            }
            self.feedback.emit();
        }
        self.dragging = false;
    }
}

impl Render for FeedbackPreview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let segments = std::mem::take(&mut self.segments);
        let caps = std::mem::take(&mut self.caps);
        let color = COLORS[self.color];
        let surface_bounds = self.surface_bounds.clone();
        let source = canvas(
            move |bounds, _, _| {
                surface_bounds.set(bounds);
            },
            move |bounds, _, window, _| {
                if !segments.is_empty() {
                    let mut path = PathBuilder::stroke(px(5.));
                    let mut end = None;
                    for segment in &segments {
                        if end != Some(segment.start) {
                            path.move_to(bounds.origin + segment.start);
                        }
                        path.curve_to(bounds.origin + segment.end, bounds.origin + segment.control);
                        end = Some(segment.end);
                    }
                    if let Ok(path) = path.build() {
                        window.paint_path(path, rgb(color));
                    }
                }
                for position in &caps {
                    let radius = 2.5;
                    window.paint_quad(quad(
                        Bounds::new(
                            bounds.origin + *position - point(px(radius), px(radius)),
                            size(px(radius * 2.), px(radius * 2.)),
                        ),
                        px(radius),
                        rgb(color),
                        px(0.),
                        gpui::transparent_black(),
                        Default::default(),
                    ));
                }
            },
        )
        .size_full();
        let content = subtree_effect_chain(
            source,
            [
                self.feedback.stage(),
                EffectStage::bloom(BloomOptions {
                    radius: px(24.),
                    intensity: 1.8,
                    ..Default::default()
                })
                .enabled(self.bloom),
            ],
        )
        .capture_padding(px(32.));

        div()
            .size_full()
            .p_8()
            .bg(rgb(0x0b101a))
            .text_color(rgb(0xe9efff))
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
                            .child(div().text_size(px(28.)).child("History feedback"))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(0x7f8ba5))
                                    .child("Draw with light."),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_3()
                            .child(
                                div()
                                    .id("pause")
                                    .px_4()
                                    .py_2()
                                    .rounded_full()
                                    .bg(rgb(0x283858))
                                    .cursor_pointer()
                                    .child(if self.feedback.is_paused() {
                                        "Resume"
                                    } else {
                                        "Pause"
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.feedback.set_paused(!this.feedback.is_paused());
                                        this.segments.clear();
                                        this.caps.clear();
                                        this.dragging = false;
                                        this.last_point = None;
                                        this.stroke_end = None;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                div()
                                    .id("clear")
                                    .px_4()
                                    .py_2()
                                    .rounded_full()
                                    .bg(rgb(0x1b2334))
                                    .cursor_pointer()
                                    .child("Clear")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.feedback.clear();
                                        this.segments.clear();
                                        this.caps.clear();
                                        this.last_point = None;
                                        this.stroke_end = None;
                                        this.dragging = false;
                                        this.has_drawn = false;
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
            .child(
                div()
                    .id("drawing-surface")
                    .relative()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .rounded(px(24.))
                    .overflow_hidden()
                    .bg(rgb(0x101725))
                    .border_1()
                    .border_color(rgba(0xffffff10))
                    .cursor_crosshair()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                            this.finish_stroke();
                            this.dragging = true;
                            this.record(event.position);
                            cx.notify();
                        }),
                    )
                    .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                        if event.pressed_button != Some(MouseButton::Left) {
                            if this.dragging {
                                this.finish_stroke();
                                cx.notify();
                            }
                            return;
                        }
                        if this.dragging && !this.feedback.is_paused() {
                            this.record(event.position);
                            cx.notify();
                        }
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.finish_stroke();
                            cx.notify();
                        }),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.finish_stroke();
                            cx.notify();
                        }),
                    )
                    .child(content)
                    .when(!self.has_drawn, |panel| {
                        panel.child(
                            div()
                                .absolute()
                                .inset_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_color(rgb(0x63718d))
                                .text_sm()
                                .child("Drag to leave a light trail"),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_6()
                    .child(div().flex().items_center().gap_3().children(
                        COLORS.into_iter().enumerate().map(|(index, color)| {
                            div()
                                .id(("color", index))
                                .size(px(30.))
                                .p(px(6.))
                                .rounded_full()
                                .border_1()
                                .border_color(if self.color == index {
                                    rgba(0xffffff80)
                                } else {
                                    rgba(0xffffff00)
                                })
                                .cursor_pointer()
                                .child(div().size_full().rounded_full().bg(rgb(color)))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.color = index;
                                    cx.notify();
                                }))
                        }),
                    ))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(div().text_sm().text_color(rgb(0x9caac4)).child(format!(
                                "Fade · {:.1} s",
                                self.feedback.fade_duration().as_secs_f32()
                            )))
                            .children([("shorter", "−", -0.2_f32), ("longer", "+", 0.2)].map(
                                |(id, label, delta)| {
                                    div()
                                        .id(id)
                                        .px_3()
                                        .py_2()
                                        .rounded(px(8.))
                                        .bg(rgb(0x1b2334))
                                        .cursor_pointer()
                                        .child(label)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            let seconds =
                                                (this.feedback.fade_duration().as_secs_f32()
                                                    + delta)
                                                    .clamp(0.2, 3.);
                                            this.feedback.set_fade_duration(
                                                Duration::from_secs_f32(seconds),
                                            );
                                            cx.notify();
                                        }))
                                },
                            )),
                    )
                    .child(
                        div()
                            .id("bloom")
                            .px_4()
                            .py_2()
                            .rounded_full()
                            .bg(rgb(0x1b2334))
                            .cursor_pointer()
                            .child(if self.bloom {
                                "Bloom · On"
                            } else {
                                "Bloom · Off"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.bloom = !this.bloom;
                                cx.notify();
                            })),
                    ),
            )
            .when(!window.supports_subtree_effects(), |root| {
                root.child(
                    div()
                        .text_sm()
                        .text_color(rgb(0x9caac4))
                        .child("Feedback is unavailable on this renderer."),
                )
            })
    }
}

fn main() {
    application().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1060.), px(760.)),
                    cx,
                ))),
                ..Default::default()
            },
            |_, cx| cx.new(|_| FeedbackPreview::new()),
        )
        .expect("failed to open feedback example");
    });
}
