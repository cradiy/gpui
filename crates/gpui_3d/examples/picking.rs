use gpui::{
    App, Bounds, Context, MouseButton, Pixels, Point, Render, Window, WindowBounds, WindowOptions,
    div, prelude::*, px, rgb, size,
};
use gpui_3d::{Camera, Material, Mesh, Object, ObjectId, Scene, viewport3d};
use gpui_platform::application;

struct CameraDrag {
    origin: Point<Pixels>,
    previous: Point<Pixels>,
    button: MouseButton,
}

struct Picking {
    yaw: f32,
    pitch: f32,
    distance: f32,
    drag: Option<CameraDrag>,
    suppress_click: bool,
    hovered: Option<ObjectId>,
    selected: Option<ObjectId>,
    _activation: gpui::Subscription,
}

impl Picking {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let activation = cx.observe_window_activation(window, |this, window, cx| {
            if !window.is_window_active() {
                this.drag = None;
                this.suppress_click = true;
                this.hovered = None;
                cx.notify();
            }
        });
        Self {
            yaw: 0.3,
            pitch: 0.25,
            distance: 7.,
            drag: None,
            suppress_click: false,
            hovered: None,
            selected: None,
            _activation: activation,
        }
    }

    fn scene(&self) -> Scene {
        let mut scene = Scene::new()
            .camera(Camera::orbit(self.yaw, self.pitch, self.distance))
            .object(
                Object::new(Mesh::plane(), Material::color(rgb(0x18283b)))
                    .rotation([-std::f32::consts::FRAC_PI_2, 0., 0.])
                    .position([0., -1., 0.])
                    .scale([8., 6., 1.]),
            );
        for (name, position, rotation, color, highlight) in [
            ("Coral", [-1.4, -0.2, 0.], [0., 0.4, 0.], 0xd58273, 0xffb09d),
            ("Ice", [0., 0., -1.2], [0.15, -0.3, 0.], 0x68b6c8, 0xa3f1ff),
            (
                "Iris",
                [1.25, -0.3, 0.5],
                [0., -0.35, 0.1],
                0x9786cd,
                0xd2bbff,
            ),
        ] {
            let id: ObjectId = name.into();
            let active = self.hovered.as_ref() == Some(&id);
            let selected = self.selected.as_ref() == Some(&id);
            let material = Material::color(rgb(if active {
                highlight
            } else if selected {
                0xf2dbaa
            } else {
                color
            }));
            scene = scene.object(
                Object::new(Mesh::cube(), material)
                    .id(id)
                    .position(position)
                    .rotation(rotation)
                    .scale([1.4; 3]),
            );
        }
        scene
    }

    fn label(id: &Option<ObjectId>) -> &'static str {
        ["Coral", "Ice", "Iris"]
            .into_iter()
            .find(|name| id.as_ref() == Some(&ObjectId::from(*name)))
            .unwrap_or("None")
    }
}

impl Render for Picking {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .p_6()
            .bg(rgb(0x0c1422))
            .text_color(rgb(0xe6eefc))
            .flex()
            .flex_col()
            .gap_4()
            .child(div().text_size(px(30.)).child("Within reach"))
            .child(
                div()
                    .text_color(rgb(0x96a9c6))
                    .child("Hover to highlight · Click to select · Drag to orbit · Scroll to zoom"),
            )
            .child(
                div()
                    .id("camera-controls")
                    .relative()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .bg(rgb(0x111e30))
                    .rounded(px(24.))
                    .overflow_hidden()
                    .capture_any_mouse_down(cx.listener(
                        |this, event: &gpui::MouseDownEvent, _, _| {
                            if matches!(event.button, MouseButton::Left | MouseButton::Right) {
                                this.suppress_click = false;
                                this.drag = Some(CameraDrag {
                                    origin: event.position,
                                    previous: event.position,
                                    button: event.button,
                                });
                            }
                        },
                    ))
                    .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                        if let Some(drag) = &mut this.drag {
                            if event.pressed_button == Some(drag.button) {
                                if !this.suppress_click
                                    && (event.position - drag.origin).magnitude() <= 4.
                                {
                                    return;
                                }
                                this.suppress_click = true;
                                let delta = event.position - drag.previous;
                                this.yaw -= f32::from(delta.x) * 0.008;
                                this.pitch =
                                    (this.pitch + f32::from(delta.y) * 0.008).clamp(-1.3, 1.3);
                                drag.previous = event.position;
                                this.hovered = None;
                                cx.notify();
                            } else {
                                this.drag = None;
                            }
                        }
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _, _, _| this.drag = None),
                    )
                    .on_mouse_up(
                        MouseButton::Right,
                        cx.listener(|this, _, _, _| this.drag = None),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|this, _, _, _| this.drag = None),
                    )
                    .on_mouse_up_out(
                        MouseButton::Right,
                        cx.listener(|this, _, _, _| this.drag = None),
                    )
                    .on_hover(cx.listener(|this, hovered: &bool, _, _| {
                        if !hovered {
                            if this.drag.is_some() {
                                this.suppress_click = true;
                            }
                            this.drag = None;
                        }
                    }))
                    .on_scroll_wheel(cx.listener(|this, event: &gpui::ScrollWheelEvent, _, cx| {
                        this.distance = (this.distance
                            * (f32::from(event.delta.pixel_delta(px(20.)).y) * 0.002).exp())
                        .clamp(3., 14.);
                        this.hovered = None;
                        cx.notify();
                    }))
                    .child(
                        viewport3d("objects", self.scene())
                            .size_full()
                            .on_object_hover(cx.listener(
                                |this, hit: &Option<gpui_3d::Hit>, _, cx| {
                                    let hovered = if this.drag.is_some() {
                                        None
                                    } else {
                                        hit.as_ref().and_then(|hit| hit.object_id.clone())
                                    };
                                    if this.hovered != hovered {
                                        this.hovered = hovered;
                                        cx.notify();
                                    }
                                },
                            ))
                            .on_object_click(cx.listener(|this, hit: &gpui_3d::Hit, _, cx| {
                                if !this.suppress_click {
                                    this.selected = hit.object_id.clone();
                                    cx.notify();
                                }
                            })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(format!(
                        "Hovered: {}   /   Selected: {}",
                        Self::label(&self.hovered),
                        Self::label(&self.selected)
                    ))
                    .child(
                        div()
                            .id("clear-selection")
                            .px_4()
                            .py_2()
                            .rounded_full()
                            .bg(rgb(0x263b54))
                            .cursor_pointer()
                            .child("Clear selection")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.selected = None;
                                cx.notify();
                            })),
                    ),
            )
            .when(!window.supports_scene3d(), |root| {
                root.child("3D viewports are unavailable on this renderer.")
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
            |window, cx| cx.new(|cx| Picking::new(window, cx)),
        )
        .expect("failed to open picking example");
    });
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use super::*;
    use gpui::{
        AppContext, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PlatformInput, TestAppContext,
        point,
    };

    #[test]
    fn orbit_drag_survives_redraw_and_stops_on_release() {
        for button in [MouseButton::Left, MouseButton::Right] {
            check_orbit_drag(button);
        }
    }

    fn check_orbit_drag(button: MouseButton) {
        let mut cx = TestAppContext::single();
        let handle = cx.add_window(Picking::new);
        cx.update_window(handle.into(), |_, window, cx| {
            window.draw(cx).clear();
            let start = point(px(250.), px(250.));
            window.dispatch_event(
                PlatformInput::MouseMove(MouseMoveEvent {
                    position: start,
                    ..Default::default()
                }),
                cx,
            );
            window.dispatch_event(
                PlatformInput::MouseDown(MouseDownEvent {
                    position: start,
                    button,
                    ..Default::default()
                }),
                cx,
            );
        })
        .unwrap();
        handle
            .update(&mut cx, |view, _, _| assert!(view.drag.is_some()))
            .unwrap();
        cx.update_window(handle.into(), |_, window, cx| {
            window.draw(cx).clear();
            window.dispatch_event(
                PlatformInput::MouseMove(MouseMoveEvent {
                    position: point(px(252.), px(251.)),
                    pressed_button: Some(button),
                    ..Default::default()
                }),
                cx,
            );
        })
        .unwrap();
        handle
            .update(&mut cx, |view, _, _| {
                assert_eq!((view.yaw, view.pitch), (0.3, 0.25));
                assert!(!view.suppress_click);
            })
            .unwrap();
        cx.update_window(handle.into(), |_, window, cx| {
            window.dispatch_event(
                PlatformInput::MouseMove(MouseMoveEvent {
                    position: point(px(300.), px(280.)),
                    pressed_button: Some(button),
                    ..Default::default()
                }),
                cx,
            );
        })
        .unwrap();
        handle
            .update(&mut cx, |view, _, _| {
                assert!((view.yaw - 0.3).abs() > 0.1);
                assert!((view.pitch - 0.25).abs() > 0.1);
                assert!(view.suppress_click);
            })
            .unwrap();
        cx.update_window(handle.into(), |_, window, cx| {
            window.draw(cx).clear();
            window.dispatch_event(
                PlatformInput::MouseMove(MouseMoveEvent {
                    position: point(px(250.), px(250.)),
                    pressed_button: Some(button),
                    ..Default::default()
                }),
                cx,
            );
        })
        .unwrap();
        handle
            .update(&mut cx, |view, _, _| assert!(view.suppress_click))
            .unwrap();
        cx.update_window(handle.into(), |_, window, cx| {
            window.dispatch_event(
                PlatformInput::MouseUp(MouseUpEvent {
                    position: point(px(250.), px(250.)),
                    button,
                    ..Default::default()
                }),
                cx,
            );
        })
        .unwrap();
        let angles = handle
            .update(&mut cx, |view, _, _| {
                assert!(view.drag.is_none());
                assert!(view.suppress_click);
                (view.yaw, view.pitch)
            })
            .unwrap();
        cx.update_window(handle.into(), |_, window, cx| {
            window.dispatch_event(
                PlatformInput::MouseMove(MouseMoveEvent {
                    position: point(px(350.), px(310.)),
                    ..Default::default()
                }),
                cx,
            );
        })
        .unwrap();
        handle
            .update(&mut cx, |view, _, _| {
                assert_eq!((view.yaw, view.pitch), angles)
            })
            .unwrap();
        cx.update_window(handle.into(), |_, window, _| window.activate_window())
            .unwrap();
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.dispatch_event(
                PlatformInput::MouseDown(MouseDownEvent {
                    position: point(px(250.), px(250.)),
                    button,
                    ..Default::default()
                }),
                cx,
            );
        })
        .unwrap();
        handle
            .update(&mut cx, |view, _, _| {
                assert!(view.drag.is_some());
                assert!(!view.suppress_click);
            })
            .unwrap();
        let other = cx.add_window(Picking::new);
        cx.update_window(other.into(), |_, window, _| window.activate_window())
            .unwrap();
        cx.run_until_parked();
        handle
            .update(&mut cx, |view, _, _| assert!(view.drag.is_none()))
            .unwrap();
    }
}
