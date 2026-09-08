use std::{cell::Cell, rc::Rc, sync::Arc, time::Instant};

use gpui::{
    App, Bounds, Context, Image, ImageFormat, ImageSource, MouseMoveEvent, ObjectFit, PathBuilder,
    Pixels, Point, Render, Window, WindowBounds, WindowOptions, canvas, div, img, point,
    prelude::*, px, rgb, rgba, size,
};
use gpui_effects::{LensOptions, subtree_lens};
use gpui_platform::application;

struct LensPreview {
    cover: ImageSource,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    options: LensOptions,
    target: Point<f32>,
    center: Point<f32>,
    log_scale: f32,
    hovered: bool,
    last_frame: Instant,
}

impl LensPreview {
    fn new() -> Self {
        Self {
            cover: Arc::new(Image::from_bytes(
                ImageFormat::Svg,
                include_bytes!("album-cover.svg").to_vec(),
            ))
            .into(),
            bounds: Rc::new(Cell::new(Bounds::default())),
            options: LensOptions::default(),
            target: point(0.5, 0.5),
            center: point(0.5, 0.5),
            log_scale: 0.,
            hovered: false,
            last_frame: Instant::now(),
        }
    }

    fn track(&mut self, position: Point<Pixels>) {
        let bounds = self.bounds.get();
        if bounds.size.width <= px(0.) || bounds.size.height <= px(0.) {
            return;
        }
        let local = position - bounds.origin;
        self.target = point(
            (f32::from(local.x) / f32::from(bounds.size.width)).clamp(0., 1.),
            (f32::from(local.y) / f32::from(bounds.size.height)).clamp(0., 1.),
        );
        if self.log_scale.abs() < 0.0001 {
            self.center = self.target;
        }
    }

    fn content(&self) -> impl IntoElement {
        let bounds = self.bounds.clone();
        div()
            .relative()
            .size_full()
            .bg(rgb(0x151c2d))
            .child(
                canvas(
                    move |region, _, _| bounds.set(region),
                    |region, _, window, _| {
                        let mut grid = PathBuilder::stroke(px(1.));
                        let mut x = px(0.);
                        while x < region.size.width {
                            grid.move_to(region.origin + point(x, px(0.)));
                            grid.line_to(region.origin + point(x, region.size.height));
                            x += px(48.);
                        }
                        let mut y = px(0.);
                        while y < region.size.height {
                            grid.move_to(region.origin + point(px(0.), y));
                            grid.line_to(region.origin + point(region.size.width, y));
                            y += px(48.);
                        }
                        if let Ok(path) = grid.build() {
                            window.paint_path(path, rgba(0x9db9f010));
                        }
                    },
                )
                .absolute()
                .inset_0()
                .size_full(),
            )
            .child(
                div()
                    .relative()
                    .size_full()
                    .p_10()
                    .flex()
                    .items_center()
                    .gap_8()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_4()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(0x91a8cf))
                                    .child("LOCAL / OPTICS"),
                            )
                            .child(div().text_size(px(76.)).child("Closer."))
                            .child(
                                div()
                                    .text_size(px(24.))
                                    .text_color(rgb(0xcab8f5))
                                    .child("细节，近在眼前"),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(0x98a6bf))
                                    .child("Move across the type, artwork and fine lines."),
                            )
                            .children(["01   Light", "02   Texture", "03   Perspective"].map(
                                |label| {
                                    div()
                                        .mt_2()
                                        .pt_3()
                                        .border_t_1()
                                        .border_color(rgba(0xc0d0ff20))
                                        .text_color(rgb(0xc5d2eb))
                                        .child(label)
                                },
                            )),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_4()
                            .child(
                                img(self.cover.clone())
                                    .w_full()
                                    .h(px(320.))
                                    .rounded(px(20.))
                                    .object_fit(ObjectFit::Cover),
                            )
                            .child(
                                div()
                                    .flex()
                                    .justify_between()
                                    .text_sm()
                                    .text_color(rgb(0xaab8d1))
                                    .child("Midnight garden")
                                    .child("VOL. 04"),
                            ),
                    ),
            )
    }
}

impl Render for LensPreview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32().min(0.05);
        self.last_frame = now;
        let blend = 1. - (-18. * dt).exp();
        self.center = self.center + (self.target - self.center) * blend;
        let target_scale = if self.hovered {
            self.options.magnification.ln()
        } else {
            0.
        };
        self.log_scale += (target_scale - self.log_scale) * blend;
        let distance = self.target - self.center;
        let bounds = self.bounds.get();
        let moving = distance.x.abs() * f32::from(bounds.size.width) > 0.1
            || distance.y.abs() * f32::from(bounds.size.height) > 0.1;
        let changing = (target_scale - self.log_scale).abs() > 0.0001;
        if !moving {
            self.center = self.target;
        }
        if !changing {
            self.log_scale = target_scale;
        }
        if (moving || changing) && window.supports_subtree_effects() {
            window.request_animation_frame();
        }
        let surface = subtree_lens(
            self.content(),
            LensOptions {
                center: self.center,
                magnification: self.log_scale.exp(),
                ..self.options
            },
        );
        div()
            .size_full()
            .p_8()
            .bg(rgb(0x0b101a))
            .text_color(rgb(0xf0edff))
            .flex()
            .flex_col()
            .gap_6()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().text_size(px(28.)).child("Local lens"))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(0x8995b1))
                            .child("A closer look, a softer edge."),
                    ),
            )
            .child(
                div()
                    .id("lens-surface")
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .rounded(px(24.))
                    .overflow_hidden()
                    .on_hover(cx.listener(|this, hovered: &bool, window, cx| {
                        this.hovered = *hovered;
                        if *hovered {
                            this.track(window.mouse_position());
                        }
                        this.last_frame = Instant::now();
                        cx.notify();
                    }))
                    .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                        this.track(event.position);
                        cx.notify();
                    }))
                    .child(surface),
            )
            .child(
                div()
                    .flex()
                    .justify_between()
                    .gap_4()
                    .child(
                        div().flex().gap_3().children(
                            [("Magnify", 1.8_f32), ("Compress", 0.65)]
                                .into_iter()
                                .enumerate()
                                .map(|(index, (label, scale))| {
                                    div()
                                        .id(("mode", index))
                                        .px_4()
                                        .py_2()
                                        .rounded_full()
                                        .bg(rgb(if self.options.magnification == scale {
                                            0x38486c
                                        } else {
                                            0x182133
                                        }))
                                        .cursor_pointer()
                                        .child(label)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.options.magnification = scale;
                                            cx.notify();
                                        }))
                                }),
                        ),
                    )
                    .child(
                        div().flex().gap_3().children(
                            [("Small", 150.), ("Medium", 220.), ("Wide", 320.)]
                                .into_iter()
                                .enumerate()
                                .map(|(index, (label, radius))| {
                                    div()
                                        .id(("radius", index))
                                        .px_4()
                                        .py_2()
                                        .rounded_full()
                                        .bg(rgb(if self.options.radius == px(radius) {
                                            0x38486c
                                        } else {
                                            0x182133
                                        }))
                                        .cursor_pointer()
                                        .child(label)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.options.radius = px(radius);
                                            cx.notify();
                                        }))
                                }),
                        ),
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
                    size(px(1100.), px(780.)),
                    cx,
                ))),
                ..Default::default()
            },
            |_, cx| cx.new(|_| LensPreview::new()),
        )
        .expect("failed to open lens example");
    });
}
