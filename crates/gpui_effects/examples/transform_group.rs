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
    raster_scale: Option<f32>,
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
        let viewport = transform_group(content, matrix);
        let viewport = match self.raster_scale {
            Some(scale) => viewport.raster_scale(scale),
            None => viewport.auto_raster_scale("canvas-raster"),
        };
        let raster_label = match self.raster_scale {
            None => "Raster: Auto",
            Some(1.) => "Raster: 1×",
            Some(2.) => "Raster: 2×",
            _ => "Raster: 4×",
        };
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
                    ["−", "+", "←", "→", "Rotate", raster_label, "Reset"]
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
                                            this.raster_scale = match this.raster_scale {
                                                None => Some(1.),
                                                Some(1.) => Some(2.),
                                                Some(2.) => Some(4.),
                                                _ => None,
                                            }
                                        }
                                        _ => {
                                            this.zoom = 1.;
                                            this.pan = 0.;
                                            this.angle = 0.;
                                            this.raster_scale = None;
                                        }
                                    }
                                    cx.notify();
                                }))
                        }),
                ),
            )
            .child(
                div()
                    .id("transform-viewport")
                    .automation_id("transform-viewport")
                    .w(px(600.))
                    .h(px(360.))
                    .bg(rgb(0x1c2733))
                    .child(viewport),
            )
            .child(div().text_sm().text_color(rgb(0x99aabc)).child(format!(
                "Zoom {:.0}% · Pan {:.0}px · Rotation {:.0}° · {}",
                self.zoom * 100.,
                self.pan,
                self.angle.to_degrees(),
                raster_label
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
                    raster_scale: None,
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

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{MouseDownEvent, MouseUpEvent, PlatformInput, TestAppContext};

    #[gpui::test]
    fn zoomed_cards_accept_clicks_across_press_redraw(cx: &mut TestAppContext) {
        for size in [size(px(780.), px(660.)), size(px(513.5), px(585.))] {
            let handle = cx.open_window(size, |_, cx| Demo {
                zoom: 1.,
                pan: 0.,
                angle: 0.,
                raster_scale: None,
                content: cx.new(|_| Content {
                    clicks: [0; 3],
                    popup: None,
                }),
            });
            cx.set_subtree_effects_supported(handle.into(), true);
            let viewport = cx
                .update_window(handle.into(), |_, window, cx| {
                    window.set_automation_enabled(true).unwrap();
                    window.draw(cx).clear();
                    let snapshot = window.automation_snapshot().unwrap();
                    let bounds = snapshot
                        .nodes
                        .iter()
                        .find(|node| node.automation_id.as_deref() == Some("transform-viewport"))
                        .unwrap()
                        .bounds
                        .unwrap();
                    window.set_automation_enabled(false).unwrap();
                    bounds
                })
                .unwrap();
            for (index, zoom) in [1., 1.2, 1.44, 1.728, 2.0736, 3.].into_iter().enumerate() {
                handle
                    .update(cx, |root, _, cx| {
                        root.zoom = zoom;
                        cx.notify();
                    })
                    .unwrap();
                cx.update_window(handle.into(), |_, window, cx| {
                    window.draw(cx).clear();
                    let position = viewport.origin + point(px(300.), px(180.));
                    window.dispatch_event(
                        PlatformInput::MouseDown(MouseDownEvent {
                            position,
                            ..Default::default()
                        }),
                        cx,
                    );
                    window.draw(cx).clear();
                    window.dispatch_event(
                        PlatformInput::MouseUp(MouseUpEvent {
                            position,
                            ..Default::default()
                        }),
                        cx,
                    );
                    window.draw(cx).clear();
                })
                .unwrap();
                handle
                    .update(cx, |root, _, cx| {
                        assert_eq!(root.content.read(cx).clicks[1], index + 1, "zoom {zoom}");
                    })
                    .unwrap();
            }
        }
    }
}
