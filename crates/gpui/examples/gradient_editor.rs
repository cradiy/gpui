use gpui::{
    App, Background, Bounds, ColorSpace, Context, GradientKind, Hsla, MouseButton, Pixels, Point,
    Render, Window, WindowBounds, WindowOptions, canvas, div, fill, linear_color_stop,
    multi_linear_gradient, point, prelude::*, px, rgb, size,
};
use std::{cell::Cell, rc::Rc};

#[derive(Clone, Copy)]
enum Drag {
    Stop,
    Hue,
    Midpoint,
}

struct GradientEditor {
    gradient: Background,
    selected: usize,
    dragging: Option<Drag>,
    tracks: [Rc<Cell<Bounds<Pixels>>>; 3],
    midpoint: f32,
    color_space: ColorSpace,
}

impl GradientEditor {
    fn new() -> Self {
        Self {
            gradient: multi_linear_gradient(
                90.,
                std::array::from_fn::<_, 20, _>(|i| {
                    linear_color_stop(
                        Hsla {
                            h: i as f32 / 22.,
                            s: 0.75,
                            l: 0.58,
                            a: 1.,
                        },
                        i as f32 / 19.,
                    )
                }),
            ),
            selected: 9,
            dragging: None,
            tracks: std::array::from_fn(|_| Rc::new(Cell::new(Bounds::default()))),
            midpoint: 0.5,
            color_space: ColorSpace::Srgb,
        }
    }

    fn edit(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(drag) = self.dragging else {
            return;
        };
        let track = self.tracks[match drag {
            Drag::Stop => 0,
            Drag::Hue => 1,
            Drag::Midpoint => 2,
        }]
        .get();
        if track.size.width <= px(0.) {
            return;
        }
        let value = ((position.x - track.origin.x) / track.size.width).clamp(0., 1.);
        let stops = self.gradient.gradient_stops();
        let mut stop = stops[self.selected];
        match drag {
            Drag::Stop => {
                let min = self
                    .selected
                    .checked_sub(1)
                    .map_or(0., |i| stops[i].percentage);
                let max = stops.get(self.selected + 1).map_or(1., |s| s.percentage);
                stop.percentage = value.clamp(min, max);
                self.gradient.set_gradient_stop(self.selected, stop);
            }
            Drag::Hue => {
                stop.color.h = value;
                self.gradient.set_gradient_stop(self.selected, stop);
            }
            Drag::Midpoint => {
                self.midpoint = value.clamp(0.01, 0.99);
                if self.selected < 19 {
                    self.gradient = self
                        .gradient
                        .clone()
                        .gradient_midpoint(self.selected, self.midpoint);
                }
            }
        }
        cx.notify();
    }
}

impl Render for GradientEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.selected;
        let stops = self.gradient.gradient_stops().to_vec();
        let handles = stops.clone();
        let geometry = self.tracks[0].clone();
        let preview = self.gradient.clone().color_space(self.color_space);
        let space = self.color_space;
        let hue = stops[selected].color.h;
        let midpoint = self.midpoint;
        div()
            .size_full()
            .bg(rgb(0x111821))
            .text_color(rgb(0xe6edf5))
            .p_8()
            .flex()
            .flex_col()
            .gap_5()
            .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                if event.pressed_button == Some(MouseButton::Left) {
                    this.edit(event.position, cx);
                } else {
                    this.dragging = None;
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.dragging = None),
            )
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(div().text_2xl().child("Gradient studio"))
                    .child(
                        div()
                            .id("space")
                            .px_4()
                            .py_2()
                            .rounded_lg()
                            .bg(rgb(0x2b394a))
                            .cursor_pointer()
                            .child(format!("{space}"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.color_space = match this.color_space {
                                    ColorSpace::Srgb => ColorSpace::Oklab,
                                    ColorSpace::Oklab => ColorSpace::Srgb,
                                };
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0x93a4b8))
                    .child("20 stops · Drag a stop to move it. Adjust its hue and midpoint below."),
            )
            .child(div().h(px(230.)).w_full().rounded_xl().bg(preview.clone()))
            .child(
                div()
                    .id("stops")
                    .h(px(36.))
                    .w_full()
                    .cursor_pointer()
                    .child(
                        canvas(
                            move |bounds, _, _| geometry.set(bounds),
                            move |bounds, _, window, _| {
                                for (i, stop) in handles.iter().enumerate() {
                                    let position = point(
                                        bounds.origin.x + bounds.size.width * stop.percentage
                                            - px(6.),
                                        bounds.origin.y + px(4.),
                                    );
                                    let outer = Bounds::new(position, size(px(12.), px(28.)));
                                    window.paint_quad(
                                        fill(
                                            outer,
                                            if i == selected {
                                                rgb(0xffffff)
                                            } else {
                                                rgb(0x526176)
                                            },
                                        )
                                        .corner_radii(px(4.)),
                                    );
                                    window.paint_quad(
                                        fill(outer.dilate(-px(2.)), stop.color)
                                            .corner_radii(px(2.)),
                                    );
                                }
                            },
                        )
                        .size_full(),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                            let bounds = this.tracks[0].get();
                            let value = (event.position.x - bounds.origin.x) / bounds.size.width;
                            this.selected = this
                                .gradient
                                .gradient_stops()
                                .iter()
                                .enumerate()
                                .min_by(|(_, a), (_, b)| {
                                    (a.percentage - value)
                                        .abs()
                                        .total_cmp(&(b.percentage - value).abs())
                                })
                                .map_or(0, |(i, _)| i);
                            this.midpoint = this
                                .gradient
                                .gradient_midpoint_at(this.selected)
                                .unwrap_or(0.5);
                            this.dragging = Some(Drag::Stop);
                            cx.notify();
                        }),
                    ),
            )
            .child(div().text_sm().child(format!(
                "Stop {} / 20   ·   Position {:.1}%",
                selected + 1,
                stops[selected].percentage * 100.
            )))
            .children(
                [
                    (Drag::Hue, "Hue", hue),
                    (Drag::Midpoint, "Midpoint", midpoint),
                ]
                .into_iter()
                .enumerate()
                .map(|(index, (drag, label, value))| {
                    let geometry = self.tracks[index + 1].clone();
                    let background = if index == 0 {
                        multi_linear_gradient(
                            90.,
                            std::array::from_fn::<_, 7, _>(|i| {
                                linear_color_stop(
                                    Hsla {
                                        h: i as f32 / 6.,
                                        s: 1.,
                                        l: 0.5,
                                        a: 1.,
                                    },
                                    i as f32 / 6.,
                                )
                            }),
                        )
                    } else {
                        rgb(0x344356).into()
                    };
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(div().text_sm().child(label))
                        .child(
                            div()
                                .id(("adjust", index))
                                .h(px(28.))
                                .w_full()
                                .cursor_pointer()
                                .child(
                                    canvas(
                                        move |bounds, _, _| geometry.set(bounds),
                                        move |bounds, _, window, _| {
                                            window.paint_quad(
                                                fill(bounds, background.clone())
                                                    .corner_radii(px(6.)),
                                            );
                                            let marker = Bounds::new(
                                                point(
                                                    bounds.origin.x + bounds.size.width * value
                                                        - px(3.),
                                                    bounds.origin.y - px(2.),
                                                ),
                                                size(px(6.), bounds.size.height + px(4.)),
                                            );
                                            window.paint_quad(
                                                fill(marker, rgb(0xffffff)).corner_radii(px(3.)),
                                            );
                                        },
                                    )
                                    .size_full(),
                                )
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(
                                        move |this, event: &gpui::MouseDownEvent, _, cx| {
                                            this.dragging = Some(drag);
                                            this.edit(event.position, cx);
                                        },
                                    ),
                                ),
                        )
                }),
            )
            .child(
                div().flex().gap_4().h(px(110.)).children(
                    [
                        GradientKind::Radial,
                        GradientKind::Angular,
                        GradientKind::Diamond,
                    ]
                    .into_iter()
                    .map(|kind| {
                        div()
                            .flex_1()
                            .h_full()
                            .rounded_lg()
                            .bg(preview.clone().gradient_kind(kind))
                    }),
                ),
            )
    }
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(940.), px(850.)),
                    cx,
                ))),
                ..Default::default()
            },
            |_, cx| cx.new(|_| GradientEditor::new()),
        )
        .unwrap();
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        cx.activate(true);
    });
}
