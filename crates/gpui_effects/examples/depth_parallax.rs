use std::{cell::Cell, path::PathBuf, rc::Rc, sync::Arc, time::Instant};

use gpui::{
    App, Bounds, Context, Image, ImageFormat, ImageSource, Pixels, Point, Render, Window,
    WindowBounds, WindowOptions, canvas, div, point, prelude::*, px, rgb, size,
};
use gpui_effects::{DepthParallaxOptions, depth_parallax};
use gpui_platform::application;

struct DepthPreview {
    color: ImageSource,
    depth: ImageSource,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    offset: Point<f32>,
    target: Point<f32>,
    options: DepthParallaxOptions,
    enabled: bool,
    show_depth: bool,
    last_frame: Instant,
}

impl DepthPreview {
    fn new() -> Self {
        let args = std::env::args_os().skip(1).collect::<Vec<_>>();
        let (color, depth) = if args.len() == 2 {
            (
                ImageSource::from(PathBuf::from(&args[0])),
                ImageSource::from(PathBuf::from(&args[1])),
            )
        } else {
            assert!(
                args.is_empty(),
                "usage: depth_parallax [color-image depth-map]"
            );
            let svg = |bytes: &[u8]| {
                ImageSource::from(Arc::new(Image::from_bytes(
                    ImageFormat::Svg,
                    bytes.to_vec(),
                )))
            };
            (
                svg(include_bytes!("depth-landscape.svg")),
                svg(include_bytes!("depth-landscape-map.svg")),
            )
        };
        Self {
            color,
            depth,
            bounds: Rc::new(Cell::new(Bounds::default())),
            offset: point(0., 0.),
            target: point(0., 0.),
            options: DepthParallaxOptions::default(),
            enabled: true,
            show_depth: false,
            last_frame: Instant::now(),
        }
    }
}

impl Render for DepthPreview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32().min(0.05);
        self.last_frame = now;
        if !window.is_window_active() {
            self.target = point(0., 0.);
        }
        let target = if self.enabled {
            self.target
        } else {
            point(0., 0.)
        };
        let blend = 1. - (-dt * 12.).exp();
        self.offset.x += (target.x - self.offset.x) * blend;
        self.offset.y += (target.y - self.offset.y) * blend;
        if (target.x - self.offset.x).hypot(target.y - self.offset.y) < 0.0005 {
            self.offset = target;
        } else if window.is_window_active() {
            window.request_animation_frame();
        }
        let bounds = self.bounds.clone();
        div()
            .size_full()
            .p_6()
            .bg(rgb(0x0d1524))
            .text_color(rgb(0xe8edf7))
            .flex()
            .flex_col()
            .items_center()
            .gap_5()
            .child(
                div()
                    .w_full()
                    .max_w(px(1120.))
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(div().text_size(px(30.)).child("Quiet dimensions"))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(0x94a7c1))
                                    .child("Move across the landscape · Near and far"),
                            ),
                    )
                    .child(
                        div().flex().gap_2().children(
                            [
                                ("Parallax", self.enabled),
                                ("Depth map", self.show_depth),
                                ("Invert", self.options.invert_depth),
                            ]
                            .into_iter()
                            .enumerate()
                            .map(|(index, (label, active))| {
                                div()
                                    .id(("mode", index))
                                    .px_4()
                                    .py_2()
                                    .rounded_full()
                                    .cursor_pointer()
                                    .bg(rgb(if active { 0x3e526f } else { 0x202d42 }))
                                    .child(label)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        match index {
                                            0 => this.enabled = !this.enabled,
                                            1 => this.show_depth = !this.show_depth,
                                            _ => {
                                                this.options.invert_depth =
                                                    !this.options.invert_depth
                                            }
                                        }
                                        cx.notify();
                                    }))
                            }),
                        ),
                    ),
            )
            .child(
                div()
                    .id("landscape")
                    .relative()
                    .w_full()
                    .max_w(px(1120.))
                    .flex_1()
                    .min_h_0()
                    .rounded(px(24.))
                    .overflow_hidden()
                    .bg(rgb(0x172239))
                    .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                        let bounds = this.bounds.get();
                        if bounds.size.width > px(0.) && bounds.size.height > px(0.) {
                            let local = event.position - bounds.origin;
                            this.target = point(
                                (f32::from(local.x) / f32::from(bounds.size.width) * 2. - 1.)
                                    .clamp(-1., 1.),
                                (f32::from(local.y) / f32::from(bounds.size.height) * 2. - 1.)
                                    .clamp(-1., 1.),
                            );
                            cx.notify();
                        }
                    }))
                    .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                        if !hovered {
                            this.target = point(0., 0.);
                            cx.notify();
                        }
                    }))
                    .child(
                        canvas(move |region, _, _| bounds.set(region), |_, _, _, _| {})
                            .absolute()
                            .inset_0()
                            .size_full(),
                    )
                    .child(
                        depth_parallax(
                            if self.show_depth {
                                self.depth.clone()
                            } else {
                                self.color.clone()
                            },
                            self.depth.clone(),
                            DepthParallaxOptions {
                                offset: self.offset,
                                ..self.options
                            },
                        )
                        .absolute()
                        .inset_0()
                        .size_full(),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .max_w(px(1120.))
                    .flex()
                    .justify_between()
                    .items_center()
                    .gap_3()
                    .child(
                        div().flex().gap_2().children(
                            [("Gentle", 0.025), ("Natural", 0.045), ("Deep", 0.075)]
                                .into_iter()
                                .enumerate()
                                .map(|(index, (label, strength))| {
                                    div()
                                        .id(("strength", index))
                                        .px_4()
                                        .py_2()
                                        .rounded_full()
                                        .cursor_pointer()
                                        .bg(rgb(if self.options.strength == strength {
                                            0x3e526f
                                        } else {
                                            0x202d42
                                        }))
                                        .child(label)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.options.strength = strength;
                                            cx.notify();
                                        }))
                                }),
                        ),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(0x94a7c1))
                            .child("Depth: dark → far · light → near"),
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
                    size(px(1180.), px(820.)),
                    cx,
                ))),
                ..Default::default()
            },
            |_, cx| cx.new(|_| DepthPreview::new()),
        )
        .expect("failed to open depth parallax example");
    });
}
