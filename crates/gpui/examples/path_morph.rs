use std::{cell::RefCell, rc::Rc, time::Instant};

use gpui::{
    App, Bounds, Context, FillOptions, LineCap, LineJoin, Path, PathBuilder, PathMorph, Pixels,
    Render, StrokeOptions, Window, WindowBounds, WindowOptions, canvas, div, point, prelude::*, px,
    rgb, size,
};
use gpui_platform::application;

fn shape(index: usize, target: bool) -> PathBuilder {
    let p = |x, y| point(px(x), px(y));
    let mut path = PathBuilder::fill();
    match index {
        0 => {
            let lines = if target {
                [
                    [-28., -28., 28., 28.],
                    [0., 0., 0., 0.],
                    [-28., 28., 28., -28.],
                ]
            } else {
                [
                    [-36., -22., 36., -22.],
                    [-36., 0., 36., 0.],
                    [-36., 22., 36., 22.],
                ]
            };
            for [x1, y1, x2, y2] in lines {
                path.move_to(p(x1, y1));
                path.line_to(p(x2, y2));
            }
        }
        1 => {
            if target {
                path.move_to(p(-32., 0.));
                path.line_to(p(-10., 24.));
                path.move_to(p(-10., 24.));
                path.line_to(p(8., 2.));
                path.line_to(p(36., -30.));
            } else {
                path.move_to(p(0., -34.));
                path.line_to(p(0., 28.));
                path.move_to(p(-26., 6.));
                path.line_to(p(0., 32.));
                path.line_to(p(26., 6.));
            }
        }
        _ if target => {
            let r = 38.;
            let c = 12.;
            let k = c * 0.552_284_8;
            let q = r - c;
            path.move_to(p(-q, -r));
            for [end, a, b] in [
                [p(q, -r), p(-q / 3., -r), p(q / 3., -r)],
                [p(r, -q), p(q + k, -r), p(r, -q - k)],
                [p(r, q), p(r, -q / 3.), p(r, q / 3.)],
                [p(q, r), p(r, q + k), p(q + k, r)],
                [p(-q, r), p(q / 3., r), p(-q / 3., r)],
                [p(-r, q), p(-q - k, r), p(-r, q + k)],
                [p(-r, -q), p(-r, q / 3.), p(-r, -q / 3.)],
                [p(-q, -r), p(-r, -q - k), p(-q - k, -r)],
            ] {
                path.cubic_bezier_to(end, a, b);
            }
            path.close();
        }
        _ => {
            let angles: [f32; 9] = [-120., -60., -30., 30., 60., 120., 150., 210., 240.];
            let radius = 40.;
            let start = angles[0].to_radians();
            path.move_to(p(radius * start.cos(), radius * start.sin()));
            for pair in angles.windows(2) {
                let a = pair[0].to_radians();
                let b = pair[1].to_radians();
                let k = 4. / 3. * ((b - a) / 4.).tan();
                path.cubic_bezier_to(
                    p(radius * b.cos(), radius * b.sin()),
                    p(
                        radius * (a.cos() - k * a.sin()),
                        radius * (a.sin() + k * a.cos()),
                    ),
                    p(
                        radius * (b.cos() + k * b.sin()),
                        radius * (b.sin() - k * b.cos()),
                    ),
                );
            }
            path.close();
        }
    }
    path
}

struct CachedMorph {
    bounds: Bounds<Pixels>,
    morph: PathMorph,
    frame: Option<(f32, Path<Pixels>)>,
}

struct MorphPreview {
    caches: [Rc<RefCell<Option<CachedMorph>>>; 3],
    progress: f32,
    direction: f32,
    hold: f32,
    seconds: f32,
    playing: bool,
    last_frame: Instant,
}

impl MorphPreview {
    fn new() -> Self {
        Self {
            caches: std::array::from_fn(|_| Rc::new(RefCell::new(None))),
            progress: 0.,
            direction: 1.,
            hold: 0.6,
            seconds: 1.2,
            playing: true,
            last_frame: Instant::now(),
        }
    }

    fn advance(&mut self) {
        let now = Instant::now();
        let mut dt = now.duration_since(self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;
        if !self.playing {
            return;
        }
        let waiting = self.hold.min(dt);
        self.hold -= waiting;
        dt -= waiting;
        self.progress = (self.progress + self.direction * dt / self.seconds).clamp(0., 1.);
        if (self.progress == 1. && self.direction > 0.)
            || (self.progress == 0. && self.direction < 0.)
        {
            self.direction = -self.direction;
            self.hold = 0.6;
        }
    }

    fn panel(&self, index: usize) -> impl IntoElement {
        let cache = self.caches[index].clone();
        let progress = self.progress * self.progress * (3. - 2. * self.progress);
        let playing = self.playing;
        let colors = [0x8de6f1, 0xa5e8c1, 0xb4a1f5];
        let (title, from, to) = [
            ("Navigation", "Menu", "Close"),
            ("Action", "Download", "Done"),
            ("Contour", "Circle", "Rounded square"),
        ][index];
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .rounded(px(24.))
            .bg(rgb(0x121c2d))
            .flex()
            .flex_col()
            .p_6()
            .gap_4()
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(colors[index]))
                    .child(format!("0{} / {title}", index + 1)),
            )
            .child(
                canvas(
                    move |bounds, _, _| {
                        if bounds.size.width <= px(0.) || bounds.size.height <= px(0.) {
                            return None;
                        }
                        let mut cache = cache.borrow_mut();
                        if cache.as_ref().is_none_or(|c| c.bounds != bounds) {
                            let place = |mut builder: PathBuilder| {
                                builder.scale(
                                    f32::from(bounds.size.width.min(bounds.size.height)) / 125.,
                                );
                                builder.translate(
                                    bounds.origin
                                        + point(bounds.size.width * 0.5, bounds.size.height * 0.5),
                                );
                                builder
                            };
                            *cache = Some(CachedMorph {
                                bounds,
                                morph: PathMorph::new(
                                    place(shape(index, false)),
                                    place(shape(index, true)),
                                )
                                .expect("incompatible preview geometry"),
                                frame: None,
                            });
                        }
                        let cache = cache.as_mut().unwrap();
                        if cache.frame.as_ref().is_none_or(|(p, _)| *p != progress) {
                            let path = if index == 2 {
                                cache.morph.fill(progress, &FillOptions::default())
                            } else {
                                cache.morph.stroke(
                                    progress,
                                    &StrokeOptions::default()
                                        .with_line_width(6.)
                                        .with_line_cap(LineCap::Round)
                                        .with_line_join(LineJoin::Round),
                                )
                            }
                            .expect("failed to tessellate morph");
                            cache.frame = Some((progress, path));
                        }
                        cache.frame.as_ref().map(|(_, path)| path.clone())
                    },
                    move |bounds, path, window, _| {
                        if bounds.intersect(&window.content_mask().bounds).is_empty() {
                            return;
                        }
                        if playing {
                            window.request_animation_frame();
                        }
                        if let Some(path) = path {
                            window.paint_path(path, rgb(colors[index]));
                        }
                    },
                )
                .w_full()
                .flex_1()
                .min_h_0(),
            )
            .child(
                div()
                    .flex()
                    .justify_between()
                    .text_sm()
                    .child(div().text_color(rgb(0xaab9cf)).child(from))
                    .child(div().text_color(rgb(colors[index])).child(to)),
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
        .bg(rgb(if active { 0x344962 } else { 0x1c2a3f }))
        .hover(|s| s.bg(rgb(0x3a4c68)))
        .child(label.into())
}

impl Render for MorphPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.advance();
        div()
            .size_full()
            .p_8()
            .bg(rgb(0x0a111e))
            .text_color(rgb(0xe8effb))
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
                            .child(div().text_size(px(28.)).child("Path morph"))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(0x8c9db7))
                                    .child("One outline. A different state."),
                            ),
                    )
                    .child(
                        button(
                            "play",
                            if self.playing { "Pause" } else { "Play" },
                            self.playing,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.advance();
                            this.playing = !this.playing;
                            cx.notify();
                        })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .gap_4()
                    .children((0..3).map(|i| self.panel(i))),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(0x8c9db7))
                            .child(format!("Progress · {:.0}%", self.progress * 100.)),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .children(
                                [
                                    ("start", "0%", 0.),
                                    ("middle", "50%", 0.5),
                                    ("end", "100%", 1.),
                                ]
                                .map(|(id, label, p)| {
                                    button(id, label, !self.playing && self.progress == p).on_click(
                                        cx.listener(move |this, _, _, cx| {
                                            this.playing = false;
                                            this.progress = p;
                                            this.hold = 0.;
                                            this.direction = if p == 1. { -1. } else { 1. };
                                            this.last_frame = Instant::now();
                                            cx.notify();
                                        }),
                                    )
                                }),
                            )
                            .child(button("reverse", "Reverse", false).on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.advance();
                                    this.direction = -this.direction;
                                    this.hold = 0.;
                                    cx.notify();
                                },
                            )))
                            .child(
                                button("speed", format!("Duration · {:.1}s", self.seconds), false)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.advance();
                                        this.seconds = if this.seconds < 2. { 2.4 } else { 1.2 };
                                        cx.notify();
                                    })),
                            ),
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
                    size(px(1080.), px(680.)),
                    cx,
                ))),
                ..Default::default()
            },
            |_, cx| cx.new(|_| MorphPreview::new()),
        )
        .expect("failed to open path morph example");
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_shapes_pair_and_tessellate_through_the_transition() {
        for index in 0..3 {
            let morph = PathMorph::new(shape(index, false), shape(index, true)).unwrap();
            for progress in [0., 0.25, 0.5, 0.75, 1.] {
                let path = if index == 2 {
                    morph.fill(progress, &FillOptions::default())
                } else {
                    morph.stroke(
                        progress,
                        &StrokeOptions::default()
                            .with_line_width(6.)
                            .with_line_cap(LineCap::Round)
                            .with_line_join(LineJoin::Round),
                    )
                }
                .unwrap();
                assert!(!path.vertices.is_empty());
                assert!(path.vertices.iter().all(|v| {
                    f32::from(v.xy_position.x).is_finite() && f32::from(v.xy_position.y).is_finite()
                }));
            }
        }
    }
}
