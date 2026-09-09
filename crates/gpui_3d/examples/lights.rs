use gpui::{
    App, Bounds, Context, MouseButton, Pixels, Render, Window, WindowBounds, WindowOptions, canvas,
    div, prelude::*, px, rgb, size,
};
use gpui_3d::{
    Camera, ColorOutput, Light, Material, Mesh, Object, OrbitController, PbrMaterial,
    PunctualLight, Scene, ToneMapping, viewport3d,
};
use gpui_platform::application;
use std::{cell::Cell, rc::Rc};

struct LightsDemo {
    light_xy: [f32; 2],
    distance: f32,
    cone: f32,
    fill: bool,
    controls: OrbitController,
    bounds: [Rc<Cell<Bounds<Pixels>>>; 2],
    _activation: gpui::Subscription,
}

impl LightsDemo {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            light_xy: [0., 0.],
            distance: 2.,
            cone: 0.45,
            fill: false,
            controls: OrbitController::new(Camera::orbit(0., 0., 4.5)).unwrap(),
            bounds: std::array::from_fn(|_| Rc::new(Cell::new(Bounds::default()))),
            _activation: cx.observe_window_activation(window, |this, window, _| {
                if !window.is_window_active() {
                    this.controls.cancel_drag();
                }
            }),
        }
    }

    fn scene(&self, spot: bool) -> Scene {
        let position = [self.light_xy[0], self.light_xy[1], self.distance];
        let source = if spot {
            PunctualLight::spot(position, [0., 0., -1.]).cone_angles(self.cone * 0.55, self.cone)
        } else {
            PunctualLight::point(position)
        };
        let mut lights = vec![source.color(rgb(0xffbe7a)).intensity(6.).range(Some(5.))];
        if self.fill {
            lights.extend([
                PunctualLight::directional([-0.5, 0.5, 1.])
                    .color(rgb(0x69a9ff))
                    .intensity(0.5),
                PunctualLight::point([-1., -0.7, 1.])
                    .color(rgb(0xd27bff))
                    .intensity(1.5)
                    .range(Some(3.)),
            ]);
        }
        let material = Material::color(rgb(0xd5dbe3)).pbr(PbrMaterial {
            metallic: 0.15,
            roughness: 0.45,
            ..Default::default()
        });
        Scene::new()
            .camera(self.controls.camera())
            .color_output(ColorOutput {
                tone_mapping: ToneMapping::Reinhard,
                ..Default::default()
            })
            .light(Light {
                ambient: 0.025,
                ..Default::default()
            })
            .lights(lights)
            .object(
                Object::new(Mesh::plane(), material.clone())
                    .position([0., 0., -0.3])
                    .scale([2.8, 2.8, 1.]),
            )
            .object(
                Object::new(Mesh::cube(), material.clone())
                    .position([-0.55, 0.4, -0.05])
                    .scale([0.65, 0.65, 0.5]),
            )
            .object(
                Object::new(Mesh::cube(), material)
                    .position([0.5, -0.45, -0.05])
                    .rotation([0., 0., 0.2])
                    .scale([0.7, 0.5, 0.5]),
            )
    }
}

impl Render for LightsDemo {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view_id = cx.entity_id();
        div()
            .size_full()
            .p_6()
            .flex()
            .flex_col()
            .gap_4()
            .bg(rgb(0x0c1421))
            .text_color(rgb(0xe8f0fa))
            .child(div().text_size(px(30.)).child("Pools of light"))
            .child(
                div().text_color(rgb(0x95aac5)).child(
                    "Move the pointer to move the light · Right-drag to orbit · Scroll to zoom",
                ),
            )
            .child(
                div().flex().flex_wrap().gap_3().children(
                    [
                        ("near", "Light nearer"),
                        ("far", "Light farther"),
                        ("narrow", "Narrow cone"),
                        ("wide", "Wide cone"),
                        (
                            "fill",
                            if self.fill {
                                "Remove fill lights"
                            } else {
                                "Add fill lights"
                            },
                        ),
                        ("reset", "Reset view"),
                    ]
                    .into_iter()
                    .map(|(id, label)| {
                        div()
                            .id(id)
                            .px_4()
                            .py_2()
                            .rounded(px(10.))
                            .bg(rgb(0x263d56))
                            .cursor_pointer()
                            .hover(|s| s.bg(rgb(0x365974)))
                            .child(label)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                match id {
                                    "near" => this.distance = (this.distance - 0.25).max(0.5),
                                    "far" => this.distance = (this.distance + 0.25).min(4.5),
                                    "narrow" => this.cone = (this.cone - 0.1).max(0.15),
                                    "wide" => this.cone = (this.cone + 0.1).min(1.2),
                                    "fill" => this.fill = !this.fill,
                                    _ => this
                                        .controls
                                        .set_camera(Camera::orbit(0., 0., 4.5))
                                        .unwrap(),
                                }
                                cx.notify();
                            }))
                    }),
                ),
            )
            .child(
                div()
                    .flex()
                    .gap_4()
                    .flex_1()
                    .min_h_0()
                    .children([false, true].map(|spot| {
                        let index = usize::from(spot);
                        let bounds = self.bounds[index].clone();
                        let panel = div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .child(if spot { "SPOT LIGHT" } else { "POINT LIGHT" });
                        let view = div()
                            .id(if spot { "spot" } else { "point" })
                            .relative()
                            .w_full()
                            .flex_1()
                            .min_h_0()
                            .rounded(px(20.))
                            .overflow_hidden()
                            .bg(rgb(0x142237))
                            .on_mouse_down(
                                MouseButton::Right,
                                cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                                    if this
                                        .controls
                                        .begin_drag(
                                            event.button,
                                            event.position,
                                            this.bounds[index].get(),
                                        )
                                        .unwrap_or(false)
                                    {
                                        cx.stop_propagation();
                                    }
                                }),
                            )
                            .on_mouse_move(cx.listener(
                                move |this, event: &gpui::MouseMoveEvent, _, cx| {
                                    if this
                                        .controls
                                        .update_drag(
                                            event.position,
                                            event.pressed_button,
                                            this.bounds[index].get(),
                                        )
                                        .unwrap_or(false)
                                    {
                                        cx.notify();
                                    }
                                    if this.controls.is_dragging() {
                                        cx.stop_propagation();
                                    } else if event.pressed_button.is_none() {
                                        let rect = this.bounds[index].get();
                                        if rect.size.width > px(0.) && rect.size.height > px(0.) {
                                            let x = (event.position.x - rect.origin.x)
                                                / rect.size.width;
                                            let y = (event.position.y - rect.origin.y)
                                                / rect.size.height;
                                            this.light_xy = [(x - 0.5) * 3., (0.5 - y) * 3.];
                                            cx.notify();
                                        }
                                    }
                                },
                            ))
                            .on_mouse_up(
                                MouseButton::Right,
                                cx.listener(|this, _, _, _| {
                                    this.controls.end_drag(MouseButton::Right);
                                }),
                            )
                            .on_mouse_up_out(
                                MouseButton::Right,
                                cx.listener(|this, _, _, _| {
                                    this.controls.end_drag(MouseButton::Right);
                                }),
                            )
                            .on_hover(cx.listener(|this, hovered: &bool, _, _| {
                                if !hovered {
                                    this.controls.cancel_drag();
                                }
                            }))
                            .on_scroll_wheel(cx.listener(
                                |this, event: &gpui::ScrollWheelEvent, _, cx| {
                                    if this
                                        .controls
                                        .scroll(f32::from(event.delta.pixel_delta(px(20.)).y))
                                        .unwrap_or(false)
                                    {
                                        cx.stop_propagation();
                                        cx.notify();
                                    }
                                },
                            ))
                            .child(
                                viewport3d(
                                    if spot { "spot-scene" } else { "point-scene" },
                                    self.scene(spot),
                                )
                                .size_full(),
                            )
                            .child(
                                canvas(
                                    move |rect, _, cx| {
                                        if bounds.replace(rect) != rect {
                                            cx.notify(view_id);
                                        }
                                    },
                                    |_, _, _, _| {},
                                )
                                .absolute()
                                .inset_0()
                                .size_full(),
                            );
                        panel.child(view)
                    })),
            )
            .child(div().text_color(rgb(0xa8bdd6)).child(format!(
                "Light distance {:.2} · Spot outer half-angle {:.0}° · {} direct light(s)",
                self.distance,
                self.cone.to_degrees(),
                if self.fill { 3 } else { 1 }
            )))
    }
}

fn main() {
    application().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1280.), px(800.)),
                    cx,
                ))),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| LightsDemo::new(window, cx)),
        )
        .expect("failed to open lights example");
    });
}
