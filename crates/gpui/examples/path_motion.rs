use gpui::{
    App, Bounds, Context, LineCap, LineJoin, MeasuredPath, Path, PathBuilder, Pixels, Render,
    StrokeOptions, Window, WindowBounds, WindowOptions, canvas, div, point, prelude::*, px, quad,
    rgb, size,
};
use gpui_platform::application;
use std::{cell::RefCell, rc::Rc, time::Instant};

struct CachedPath {
    bounds: Bounds<Pixels>,
    measured: MeasuredPath,
    guide: Path<Pixels>,
}
type PathCache = Rc<RefCell<Option<CachedPath>>>;

fn stroke(width: f32) -> StrokeOptions {
    StrokeOptions::default()
        .with_line_width(width)
        .with_line_cap(LineCap::Round)
        .with_line_join(LineJoin::Round)
}

fn geometry(bounds: Bounds<Pixels>) -> CachedPath {
    let p = |x, y| bounds.origin + point(bounds.size.width * x, bounds.size.height * y);
    let mut builder = PathBuilder::stroke(px(2.));
    builder.move_to(p(0.06, 0.65));
    builder.cubic_bezier_to(p(0.5, 0.5), p(0.18, -0.3), p(0.32, 1.4));
    builder.cubic_bezier_to(p(0.94, 0.35), p(0.68, -0.4), p(0.82, 1.3));
    let measured = builder.measure();
    let guide = measured
        .stroke(&stroke(1.5))
        .expect("failed to build path guide");
    CachedPath {
        bounds,
        measured,
        guide,
    }
}

struct PathPreview {
    caches: [PathCache; 3],
    seconds: f64,
    last_tick: Instant,
    paused: bool,
    reverse: bool,
    speed: f64,
}

impl PathPreview {
    fn new() -> Self {
        Self {
            caches: std::array::from_fn(|_| Rc::new(RefCell::new(None))),
            seconds: 0.,
            last_tick: Instant::now(),
            paused: false,
            reverse: false,
            speed: 1.,
        }
    }

    fn advance(&mut self) {
        let now = Instant::now();
        if !self.paused {
            self.seconds += now.duration_since(self.last_tick).as_secs_f64()
                * self.speed
                * if self.reverse { -1. } else { 1. };
        }
        self.last_tick = now;
    }

    fn row(&self, index: usize) -> impl IntoElement {
        let prepaint_cache = self.caches[index].clone();
        let paint_cache = self.caches[index].clone();
        let seconds = self.seconds;
        let paused = self.paused;
        let reverse = self.reverse;
        let colors = [0x82e8f5, 0xb5a1ff, 0xf1ba91];
        let source = canvas(
            move |bounds, _, _| {
                let mut cache = prepaint_cache.borrow_mut();
                if cache.as_ref().is_none_or(|cache| cache.bounds != bounds) {
                    *cache = Some(geometry(bounds));
                }
            },
            move |bounds, _, window, _| {
                if bounds.intersect(&window.content_mask().bounds).is_empty() {
                    return;
                }
                if !paused {
                    window.request_animation_frame();
                }
                let cache = paint_cache.borrow();
                let Some(cache) = cache.as_ref() else {
                    return;
                };
                let measured = &cache.measured;
                let length = f64::from(f32::from(measured.length()));
                if length <= 0. {
                    return;
                }
                window.paint_path(cache.guide.clone(), rgb(0x2a354b));
                match index {
                    0 => {
                        let progress = (seconds.rem_euclid(5.) / 3.8).min(1.) as f32;
                        if let Ok(path) = measured.stroke_range(0.0..progress, &stroke(3.5)) {
                            window.paint_path(path, rgb(colors[index]));
                        }
                        if progress > 0.
                            && let Some(sample) = measured.sample(progress)
                        {
                            window.paint_quad(quad(
                                Bounds::new(
                                    sample.position - point(px(4.), px(4.)),
                                    size(px(8.), px(8.)),
                                ),
                                px(4.),
                                rgb(0xc5f7ff),
                                px(0.),
                                gpui::transparent_black(),
                                Default::default(),
                            ));
                        }
                    }
                    1 => {
                        let distance = (seconds * 120.).rem_euclid(length);
                        let progress = (distance / length) as f32;
                        let tail = if reverse {
                            progress..(progress + 0.13).min(1.)
                        } else {
                            (progress - 0.13).max(0.)..progress
                        };
                        if let Ok(path) = measured.stroke_range(tail, &stroke(3.5)) {
                            window.paint_path(path, rgb(colors[index]));
                        }
                        if let Some(sample) = measured.sample_at(px(distance as f32)) {
                            let direction = if reverse { -1. } else { 1. };
                            let along = point(
                                px(sample.tangent.x * direction),
                                px(sample.tangent.y * direction),
                            );
                            let across = point(-along.y, along.x);
                            let mut head = PathBuilder::fill();
                            head.move_to(sample.position + along * 8.);
                            head.line_to(sample.position - along * 5. + across * 4.5);
                            head.line_to(sample.position - along * 2.);
                            head.line_to(sample.position - along * 5. - across * 4.5);
                            head.close();
                            if let Ok(path) = head.build() {
                                window.paint_path(path, rgb(0xe8ddff));
                            }
                        }
                    }
                    _ => {
                        if let Ok(path) = measured.stroke_dashed(
                            0.0..1.,
                            &[px(14.), px(11.)],
                            px((-seconds * 35.).rem_euclid(25.) as f32),
                            &stroke(3.),
                        ) {
                            window.paint_path(path, rgb(colors[index]));
                        }
                    }
                }
            },
        )
        .size_full();
        let labels = [
            ("01", "Reveal", "Draw by arc length"),
            ("02", "Travel", "120 px/s · Tangent aligned"),
            ("03", "Dashes", "Continuous pattern phase"),
        ];
        let (number, title, caption) = labels[index];
        div()
            .relative()
            .w_full()
            .flex_1()
            .min_h_0()
            .rounded(px(22.))
            .overflow_hidden()
            .bg(rgb(0x111a2b))
            .flex()
            .flex_col()
            .px_6()
            .pt_4()
            .pb_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .gap_3()
                            .items_center()
                            .child(div().text_xs().text_color(rgb(colors[index])).child(number))
                            .child(div().text_size(px(17.)).child(title)),
                    )
                    .child(div().text_xs().text_color(rgb(0x7f8fa9)).child(caption)),
            )
            .child(div().w_full().flex_1().min_h_0().child(source))
    }
}

fn button(
    id: &'static str,
    text: impl Into<gpui::SharedString>,
    active: bool,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .px_4()
        .py_2()
        .rounded_full()
        .cursor_pointer()
        .bg(rgb(if active { 0x334663 } else { 0x1a283d }))
        .hover(|style| style.bg(rgb(0x3a4a68)))
        .child(text.into())
}
impl Render for PathPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.advance();
        div()
            .size_full()
            .p_8()
            .bg(rgb(0x090f1b))
            .text_color(rgb(0xe7eefb))
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .mb_2()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(div().text_size(px(28.)).child("Path motion"))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(0x8c9cb7))
                                    .child("One curve. Distance, direction, and rhythm."),
                            ),
                    )
                    .child(
                        button(
                            "pause",
                            if self.paused { "Resume" } else { "Pause" },
                            self.paused,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.advance();
                            this.paused = !this.paused;
                            cx.notify();
                        })),
                    ),
            )
            .children((0..3).map(|i| self.row(i)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .mt_2()
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(0x8c9cb7))
                            .child("Arc-length sampling · Cached geometry"),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                button("speed", format!("Speed · {:.1}×", self.speed), false)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.advance();
                                        this.speed = if this.speed < 1. {
                                            1.
                                        } else if this.speed < 2. {
                                            2.
                                        } else {
                                            0.5
                                        };
                                        cx.notify();
                                    })),
                            )
                            .child(button("reverse", "Reverse", self.reverse).on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.advance();
                                    this.reverse = !this.reverse;
                                    cx.notify();
                                }),
                            ))
                            .child(button("replay", "Replay", false).on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.seconds = 0.;
                                    this.last_tick = Instant::now();
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
                    size(px(1100.), px(850.)),
                    cx,
                ))),
                ..Default::default()
            },
            |_, cx| cx.new(|_| PathPreview::new()),
        )
        .expect("failed to open path motion example");
    });
}
