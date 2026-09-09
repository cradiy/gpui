use gpui::{
    App, Bounds, Context, MouseButton, Pixels, Render, RenderImage, Window, WindowBounds,
    WindowOptions, canvas, div, prelude::*, px, rgb, size,
};
use gpui_3d::{
    Camera, ColorOutput, DiffuseEnvironment, Light, Material, MaterialTexture, Mesh, Object,
    OrbitController, PbrMaterial, Scene, ToneMapping, viewport3d,
};
use gpui_platform::application;
use std::{cell::Cell, rc::Rc, sync::Arc};

const PANELS: [[f32; 4]; 3] = [
    [-0.45, 0.5, 0.65, 0.65],
    [0.5, 0.3, 0.55, 1.0],
    [-0.35, -0.55, 0.85, 0.45],
];

fn occlusion_map() -> Arc<RenderImage> {
    let pixels = image::RgbaImage::from_fn(256, 256, |x, y| {
        let p = [
            (x as f32 + 0.5) / 256. * 2.4 - 1.2,
            1.2 - (y as f32 + 0.5) / 256. * 2.4,
        ];
        let mut visibility = 1_f32;
        for [cx, cy, width, height] in PANELS {
            let dx = ((p[0] - cx).abs() - width * 0.5).max(0.);
            let dy = ((p[1] - cy).abs() - height * 0.5).max(0.);
            let distance = dx.hypot(dy);
            visibility *= 1. - 0.9 * (-distance / 0.12).exp();
        }
        let value = (visibility * 255.).round() as u8;
        image::Rgba([value, value, value, 255])
    });
    Arc::new(RenderImage::new(vec![image::Frame::new(pixels)]))
}

struct OcclusionDemo {
    image: Arc<RenderImage>,
    environment: DiffuseEnvironment,
    strength: f32,
    direct_only: bool,
    controls: OrbitController,
    bounds: [Rc<Cell<Bounds<Pixels>>>; 2],
    _activation: gpui::Subscription,
}

impl OcclusionDemo {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            image: occlusion_map(),
            environment: DiffuseEnvironment::from_equirectangular([1, 1], &[[0.55, 0.7, 1.]])
                .unwrap(),
            strength: 1.,
            direct_only: false,
            controls: OrbitController::new(Camera::orbit(0.2, 0.15, 4.5)).unwrap(),
            bounds: std::array::from_fn(|_| Rc::new(Cell::new(Bounds::default()))),
            _activation: cx.observe_window_activation(window, |this, window, _| {
                if !window.is_window_active() {
                    this.controls.cancel_drag();
                }
            }),
        }
    }

    fn scene(&self, occluded: bool) -> Scene {
        let material = Material::color(rgb(0xd6c3a5)).pbr(PbrMaterial {
            roughness: 0.65,
            ..Default::default()
        });
        let backing = material
            .clone()
            .occlusion_texture(MaterialTexture::new(self.image.clone()))
            .occlusion_strength(if occluded { self.strength } else { 0. });
        let mut scene = Scene::new()
            .camera(self.controls.camera())
            .color_output(ColorOutput {
                tone_mapping: ToneMapping::Reinhard,
                ..Default::default()
            })
            .diffuse_environment(
                self.environment
                    .intensity(if self.direct_only { 0. } else { 1. }),
            )
            .light(Light {
                direction: [-0.4, 0.6, 1.],
                intensity: if self.direct_only { 3. } else { 0.7 },
                ambient: 0.,
                ..Default::default()
            })
            .object(
                Object::new(Mesh::plane(), backing)
                    .position([0., 0., -0.2])
                    .scale([2.4, 2.4, 1.]),
            );
        for [x, y, width, height] in PANELS {
            scene = scene.object(
                Object::new(Mesh::cube(), material.clone())
                    .position([x, y, 0.05])
                    .scale([width, height, 0.5]),
            );
        }
        scene
    }
}

impl Render for OcclusionDemo {
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
            .child(div().text_size(px(30.)).child("Contact and depth"))
            .child(
                div()
                    .text_color(rgb(0x95aac5))
                    .child("Authored contact occlusion · Right-drag to orbit · Scroll to zoom"),
            )
            .child(
                div().flex().flex_wrap().gap_3().children(
                    [
                        ("less", "AO −"),
                        ("more", "AO +"),
                        (
                            "light",
                            if self.direct_only {
                                "Mixed lighting"
                            } else {
                                "Direct only"
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
                                    "less" => this.strength = (this.strength - 0.1).max(0.),
                                    "more" => this.strength = (this.strength + 0.1).min(1.),
                                    "light" => this.direct_only = !this.direct_only,
                                    _ => this
                                        .controls
                                        .set_camera(Camera::orbit(0.2, 0.15, 4.5))
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
                    .children([false, true].map(|occluded| {
                        let index = usize::from(occluded);
                        let bounds = self.bounds[index].clone();
                        let panel = div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .child(if occluded {
                                "WITH OCCLUSION"
                            } else {
                                "WITHOUT OCCLUSION"
                            });
                        let view = div()
                            .id(if occluded { "occlusion" } else { "reference" })
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
                                    if occluded {
                                        "occlusion-scene"
                                    } else {
                                        "reference-scene"
                                    },
                                    self.scene(occluded),
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
                "AO strength {:.0}% · {}",
                self.strength * 100.,
                if self.direct_only {
                    "Direct light only"
                } else {
                    "Environment + direct light"
                }
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
            |window, cx| cx.new(|cx| OcclusionDemo::new(window, cx)),
        )
        .expect("failed to open ambient occlusion example");
    });
}
