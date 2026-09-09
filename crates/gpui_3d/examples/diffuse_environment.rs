use gpui::{
    App, Bounds, Context, MouseButton, Pixels, Render, Window, WindowBounds, WindowOptions, canvas,
    div, prelude::*, px, rgb, size,
};
use gpui_3d::{
    Camera, ColorOutput, DiffuseEnvironment, Light, Material, Mesh, Object, OrbitController,
    PbrMaterial, Scene, ToneMapping, Vertex, viewport3d,
};
use gpui_platform::application;
use std::{
    cell::Cell,
    f32::consts::{PI, TAU},
    rc::Rc,
};

fn sphere() -> Mesh {
    let (rings, segments) = (48, 96);
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for y in 0..=rings {
        let v = y as f32 / rings as f32;
        let theta = v * PI;
        for x in 0..=segments {
            let u = x as f32 / segments as f32;
            let phi = u * TAU;
            let normal = [
                theta.sin() * phi.cos(),
                theta.cos(),
                theta.sin() * phi.sin(),
            ];
            vertices.push(Vertex {
                position: normal.map(|v| v * 0.7),
                normal,
                uv: [u, v],
            });
        }
    }
    for y in 0..rings {
        for x in 0..segments {
            let a = y * (segments + 1) + x;
            let b = a + segments + 1;
            if y > 0 {
                indices.extend([a, a + 1, b]);
            }
            if y + 1 < rings {
                indices.extend([a + 1, b + 1, b]);
            }
        }
    }
    Mesh::new(vertices, indices)
}

fn environment() -> DiffuseEnvironment {
    let pixels: Vec<_> = (0..64)
        .flat_map(|y| {
            (0..128).map(move |x| {
                let theta = (y as f32 + 0.5) * PI / 64.;
                let phi = (x as f32 + 0.5) * TAU / 128. - PI;
                let direction = [
                    theta.sin() * phi.cos(),
                    theta.cos(),
                    theta.sin() * phi.sin(),
                ];
                let sky = direction[1].max(0.);
                let ground = (-direction[1]).max(0.);
                let warm = (direction[0] * 0.8 + direction[2] * 0.6).max(0.).powi(6) * 5.;
                [
                    0.08 + 0.1 * sky + 0.4 * ground + warm,
                    0.08 + 0.5 * sky + 0.15 * ground + warm * 0.4,
                    0.08 + 1.5 * sky + 0.05 * ground + warm * 0.08,
                ]
            })
        })
        .collect();
    DiffuseEnvironment::from_equirectangular([128, 64], &pixels).unwrap()
}

struct EnvironmentDemo {
    mesh: Mesh,
    environment: DiffuseEnvironment,
    rotation: f32,
    intensity: f32,
    enabled: bool,
    controls: OrbitController,
    bounds: [Rc<Cell<Bounds<Pixels>>>; 2],
    _activation: gpui::Subscription,
}

impl EnvironmentDemo {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            mesh: sphere(),
            environment: environment(),
            rotation: 0.,
            intensity: 1.,
            enabled: true,
            controls: OrbitController::new(Camera::orbit(0., 0.15, 4.5)).unwrap(),
            bounds: std::array::from_fn(|_| Rc::new(Cell::new(Bounds::default()))),
            _activation: cx.observe_window_activation(window, |this, window, _| {
                if !window.is_window_active() {
                    this.controls.cancel_drag();
                }
            }),
        }
    }

    fn scene(&self, environment: bool) -> Scene {
        let material = Material::color(rgb(0xe6e6e6)).pbr(PbrMaterial::default());
        let mut scene = Scene::new()
            .camera(self.controls.camera())
            .color_output(ColorOutput {
                tone_mapping: ToneMapping::Reinhard,
                ..Default::default()
            })
            .light(Light {
                intensity: 0.,
                ambient: if environment { 0. } else { 0.45 },
                ..Default::default()
            })
            .object(Object::new(self.mesh.clone(), material.clone()).position([-0.65, 0.15, 0.]))
            .object(
                Object::new(Mesh::cube(), material)
                    .position([0.85, -0.15, 0.])
                    .rotation([0.15, 0.3, 0.])
                    .scale([0.85; 3]),
            );
        if environment {
            scene = scene.diffuse_environment(
                self.environment
                    .rotation_y(self.rotation)
                    .intensity(if self.enabled { self.intensity } else { 0. }),
            );
        }
        scene
    }
}

impl Render for EnvironmentDemo {
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
            .child(div().text_size(px(30.)).child("Light from every direction"))
            .child(
                div()
                    .text_color(rgb(0x95aac5))
                    .child("Cool sky · Warm horizon · Right-drag to orbit · Scroll to zoom"),
            )
            .child(
                div().flex().flex_wrap().gap_3().children(
                    [
                        ("left", "Light ↶"),
                        ("right", "Light ↷"),
                        ("less", "Intensity −"),
                        ("more", "Intensity +"),
                        (
                            "toggle",
                            if self.enabled {
                                "Disable IBL"
                            } else {
                                "Enable IBL"
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
                                    "left" => this.rotation -= PI / 6.,
                                    "right" => this.rotation += PI / 6.,
                                    "less" => this.intensity = (this.intensity - 0.25).max(0.),
                                    "more" => this.intensity = (this.intensity + 0.25).min(4.),
                                    "toggle" => this.enabled = !this.enabled,
                                    _ => this
                                        .controls
                                        .set_camera(Camera::orbit(0., 0.15, 4.5))
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
                    .children([false, true].map(|environment| {
                        let index = usize::from(environment);
                        let bounds = self.bounds[index].clone();
                        let panel = div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .child(if environment {
                                "DIFFUSE ENVIRONMENT"
                            } else {
                                "UNIFORM AMBIENT"
                            });
                        let view = div()
                            .id(if environment {
                                "environment"
                            } else {
                                "ambient"
                            })
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
                                    if environment {
                                        "ibl-scene"
                                    } else {
                                        "ambient-scene"
                                    },
                                    self.scene(environment),
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
                "Light rotation {:.0}° · Intensity {:.2}× · No direct light",
                self.rotation.to_degrees().rem_euclid(360.),
                self.intensity
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
            |window, cx| cx.new(|cx| EnvironmentDemo::new(window, cx)),
        )
        .expect("failed to open diffuse environment example");
    });
}
