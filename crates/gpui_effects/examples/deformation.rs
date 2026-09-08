use std::{cell::Cell, rc::Rc, sync::Arc, time::Instant};

use gpui::{
    App, Bounds, Context, Image, ImageFormat, ImageSource, MouseButton, ObjectFit, Pixels, Point,
    Render, Window, WindowBounds, WindowOptions, canvas, div, img, point, prelude::*, px, rgb,
    size,
};
use gpui_effects::{DeformationOptions, ElasticOffset, subtree_deformation};
use gpui_platform::application;

struct DeformationPreview {
    cover: ImageSource,
    region: Rc<Cell<Bounds<Pixels>>>,
    card: Rc<Cell<Bounds<Pixels>>>,
    options: DeformationOptions,
    spring: ElasticOffset,
    drag: Option<(Point<Pixels>, Point<Pixels>)>,
    last_frame: Instant,
}

impl DeformationPreview {
    fn new() -> Self {
        Self {
            cover: Arc::new(Image::from_bytes(
                ImageFormat::Svg,
                include_bytes!("album-cover.svg").to_vec(),
            ))
            .into(),
            region: Rc::new(Cell::new(Bounds::default())),
            card: Rc::new(Cell::new(Bounds::default())),
            options: DeformationOptions::default(),
            spring: ElasticOffset::default(),
            drag: None,
            last_frame: Instant::now(),
        }
    }

    fn advance(&mut self) {
        let now = Instant::now();
        self.spring.advance(now.duration_since(self.last_frame));
        self.last_frame = now;
    }

    fn release(&mut self) {
        if self.drag.take().is_some() {
            self.spring.release();
            self.last_frame = Instant::now();
        }
    }

    fn content(&self) -> impl IntoElement {
        let region = self.region.clone();
        let card = self.card.clone();
        div()
            .relative()
            .size_full()
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
                    .w(px(700.))
                    .h(px(380.))
                    .rounded(px(32.))
                    .overflow_hidden()
                    .bg(rgb(0x1c2335))
                    .flex()
                    .child(
                        canvas(move |bounds, _, _| card.set(bounds), |_, _, _, _| {})
                            .absolute()
                            .inset_0()
                            .size_full(),
                    )
                    .child(
                        img(self.cover.clone())
                            .w(px(290.))
                            .h_full()
                            .object_fit(ObjectFit::Cover),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .p_8()
                            .flex()
                            .flex_col()
                            .justify_between()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(0xb8a5f5))
                                    .child("AFTER HOURS / VOL. 04"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_4()
                                    .child(
                                        div()
                                            .text_size(px(42.))
                                            .line_height(px(48.))
                                            .child("Midnight\ngarden"),
                                    )
                                    .child(
                                        div()
                                            .text_color(rgb(0x9ba9c1))
                                            .child("夜色之中，轻轻舒展。"),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(rgb(0xaebbd0))
                                            .child("12 tracks · 48 min"),
                                    )
                                    .child(
                                        div()
                                            .px_5()
                                            .py_3()
                                            .rounded_full()
                                            .bg(rgb(0xb8a5f5))
                                            .text_color(rgb(0x201935))
                                            .child("Play"),
                                    ),
                            ),
                    ),
            )
    }
}

impl Render for DeformationPreview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.advance();
        if !window.is_window_active() {
            self.release();
        }
        let visible = self.region.get().intersects(&window.content_mask().bounds);
        if self.spring.is_animating() && visible && window.supports_subtree_effects() {
            window.request_animation_frame();
        }
        let content = subtree_deformation(
            self.content(),
            DeformationOptions {
                offset: self.spring.offset(),
                ..self.options
            },
        );
        div()
            .id("deformation-demo")
            .size_full()
            .p_8()
            .bg(rgb(0x0b101b))
            .text_color(rgb(0xf0edff))
            .flex()
            .flex_col()
            .gap_4()
            .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                if let Some((origin, base)) = this.drag {
                    if event.pressed_button != Some(MouseButton::Left) {
                        this.release();
                    } else {
                        this.options.offset = base + event.position - origin;
                        this.spring.drag_to(this.options.constrained_offset());
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
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(div().text_size(px(28.)).child("Elastic surface"))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(0x8997b1))
                                    .child("Grab anywhere. Let it go."),
                            ),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(0xb8a5f5))
                            .child("LOCAL DEFORMATION"),
                    ),
            )
            .child(
                div()
                    .id("surface")
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                            let bounds = this.region.get();
                            if !this.card.get().contains(&event.position)
                                || bounds.size.width <= px(0.)
                                || bounds.size.height <= px(0.)
                            {
                                return;
                            }
                            this.advance();
                            if !this.spring.is_animating() {
                                let local = event.position - bounds.origin;
                                this.options.center = point(
                                    f32::from(local.x) / f32::from(bounds.size.width),
                                    f32::from(local.y) / f32::from(bounds.size.height),
                                );
                            }
                            this.drag = Some((event.position, this.spring.offset()));
                            this.spring.grab();
                            cx.notify();
                        }),
                    )
                    .child(content),
            )
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div().flex().gap_2().children(
                            [("Local", 180.), ("Soft", 260.), ("Broad", 340.)]
                                .into_iter()
                                .enumerate()
                                .map(|(index, (label, radius))| {
                                    div()
                                        .id(("radius", index))
                                        .px_4()
                                        .py_2()
                                        .rounded_full()
                                        .cursor_pointer()
                                        .bg(rgb(if self.options.radius == px(radius) {
                                            0x34415d
                                        } else {
                                            0x192235
                                        }))
                                        .child(label)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.options.radius = px(radius);
                                            this.spring.clear();
                                            this.drag = None;
                                            cx.notify();
                                        }))
                                }),
                        ),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(0x8997b1))
                            .child("Drag to stretch · Release to return"),
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
            |_, cx| cx.new(|_| DeformationPreview::new()),
        )
        .expect("failed to open deformation example");
    });
}
