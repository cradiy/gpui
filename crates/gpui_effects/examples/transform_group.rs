use gpui::{
    AnchoredPositionMode, App, Bounds, Context, Entity, MouseButton, Render, TransformationMatrix,
    Window, WindowBounds, WindowOptions, anchored, deferred, div, point, prelude::*, px, radians,
    rgb, size,
};
use gpui_effects::transform_group;
use gpui_platform::application;

struct Demo {
    zoom: f32,
    pan: f32,
    angle: f32,
    raster_scale: f32,
    content: Entity<Content>,
}

struct Content {
    clicks: [usize; 3],
    popup: Option<usize>,
}

impl Render for Content {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .relative()
            .w(px(600.))
            .h(px(360.))
            .children((0..3).map(|index| {
                let card = div()
                    .id(("card", index))
                    .absolute()
                    .left(px(40. + index as f32 * 180.))
                    .top(px(130.))
                    .w(px(160.))
                    .h(px(100.))
                    .p_4()
                    .rounded_xl()
                    .bg(rgb([0x294c59, 0x514768, 0x68503e][index]))
                    .border_1()
                    .border_color(rgb(0x718291))
                    .cursor_pointer()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(format!("Card {}", index + 1))
                    .child(
                        div()
                            .text_sm()
                            .child(format!("{} clicks", self.clicks[index])),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.clicks[index] += 1;
                        cx.notify();
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, _, _, cx| {
                            this.popup = Some(index);
                            cx.notify();
                        }),
                    );
                if self.popup == Some(index) {
                    card.child(deferred(
                        anchored()
                            .map_anchor(true)
                            .position_mode(AnchoredPositionMode::Local)
                            .position(point(px(0.), px(100.)))
                            .offset(point(px(0.), px(8.)))
                            .child(
                                div()
                                    .id(("menu", index))
                                    .w(px(180.))
                                    .p_3()
                                    .rounded_lg()
                                    .bg(rgb(0x34445a))
                                    .text_sm()
                                    .cursor_pointer()
                                    .child("Reset counter")
                                    .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                        this.popup = None;
                                        cx.notify();
                                    }))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.clicks[index] = 0;
                                        this.popup = None;
                                        cx.stop_propagation();
                                        cx.notify();
                                    })),
                            ),
                    ))
                } else {
                    card
                }
            }))
    }
}

impl Render for Demo {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let matrix = TransformationMatrix::unit()
            .translate(point(px(300. + self.pan), px(180.)).scale(1.))
            .rotate(radians(self.angle))
            .scale(size(self.zoom, self.zoom))
            .translate(point(px(-300.), px(-180.)).scale(1.));
        let content = self
            .content
            .clone()
            .cached(div().w(px(600.)).h(px(360.)).style().clone())
            .cache_across_transforms();
        div()
            .size_full()
            .p_8()
            .flex()
            .flex_col()
            .gap_5()
            .bg(rgb(0x111821))
            .text_color(rgb(0xe8edf4))
            .child(div().text_size(px(26.)).child("Transform group"))
            .child(
                div().text_sm().text_color(rgb(0x99aabc)).child(
                    "Click a card to count. Right-click for a menu that follows its anchor.",
                ),
            )
            .child(
                div().flex().gap_2().children(
                    ["−", "+", "←", "→", "Rotate", "Raster 1× / 2×", "Reset"]
                        .into_iter()
                        .enumerate()
                        .map(|(index, label)| {
                            div()
                                .id(("control", index))
                                .px_4()
                                .py_2()
                                .rounded_lg()
                                .bg(rgb(0x293748))
                                .cursor_pointer()
                                .child(label)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    match index {
                                        0 => this.zoom = (this.zoom / 1.2).max(0.5),
                                        1 => this.zoom = (this.zoom * 1.2).min(3.),
                                        2 => this.pan -= 20.,
                                        3 => this.pan += 20.,
                                        4 => this.angle += std::f32::consts::PI / 12.,
                                        5 => {
                                            this.raster_scale =
                                                if this.raster_scale == 1. { 2. } else { 1. }
                                        }
                                        _ => {
                                            this.zoom = 1.;
                                            this.pan = 0.;
                                            this.angle = 0.;
                                        }
                                    }
                                    cx.notify();
                                }))
                        }),
                ),
            )
            .child(
                div()
                    .w(px(600.))
                    .h(px(360.))
                    .bg(rgb(0x1c2733))
                    .child(transform_group(content, matrix).raster_scale(self.raster_scale)),
            )
            .child(div().text_sm().text_color(rgb(0x99aabc)).child(format!(
                "Zoom {:.0}% · Pan {:.0}px · Rotation {:.0}° · Requested raster {:.0}×",
                self.zoom * 100.,
                self.pan,
                self.angle.to_degrees(),
                self.raster_scale
            )))
    }
}

fn main() {
    application().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(780.), px(660.)),
                    cx,
                ))),
                ..Default::default()
            },
            |_, cx| {
                cx.new(|cx| Demo {
                    zoom: 1.,
                    pan: 0.,
                    angle: 0.,
                    raster_scale: 2.,
                    content: cx.new(|_| Content {
                        clicks: [0; 3],
                        popup: None,
                    }),
                })
            },
        )
        .unwrap();
        cx.activate(true);
    });
}
