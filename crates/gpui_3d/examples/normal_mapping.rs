use gpui::{
    App, Bounds, Context, MouseButton, Pixels, Render, RenderImage, Window, WindowBounds,
    WindowOptions, canvas, div, prelude::*, px, rgb, size,
};
use gpui_3d::{
    Camera, ColorOutput, Light, Material, MaterialTexture, Mesh, Object, OrbitController,
    PbrMaterial, Scene, TextureAddressMode, TextureSampling, ToneMapping, UvTransform, viewport3d,
};
use gpui_platform::application;
use std::{cell::Cell, rc::Rc, sync::Arc};

struct Normals {
    image: Arc<RenderImage>,
    controls: OrbitController,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    strength: f32,
    density: f32,
    mirrored: bool,
    _activation: gpui::Subscription,
}

impl Normals {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let pixels = image::RgbaImage::from_fn(128, 128, |x, y| {
            let u = (x as f32 + 0.5) / 128. * std::f32::consts::TAU;
            let v = (y as f32 + 0.5) / 128. * std::f32::consts::TAU;
            let n = [-0.7 * u.cos() * v.sin(), -0.7 * u.sin() * v.cos(), 1.];
            let length = n.iter().map(|v| v * v).sum::<f32>().sqrt();
            let n = n.map(|v| ((v / length * 0.5 + 0.5) * 255.).round() as u8);
            image::Rgba([n[0], n[1], n[2], 255])
        });
        Self {
            image: Arc::new(RenderImage::new(vec![image::Frame::new(pixels)])),
            controls: OrbitController::new(Camera::orbit(0., 0.15, 4.)).unwrap(),
            bounds: Rc::new(Cell::new(Bounds::default())),
            strength: 1.,
            density: 4.,
            mirrored: false,
            _activation: cx.observe_window_activation(window, |this, window, _| {
                if !window.is_window_active() {
                    this.controls.cancel_drag();
                }
            }),
        }
    }

    fn scene(&self) -> Scene {
        let material = Material::color(rgb(0x79a6b0)).pbr(PbrMaterial {
            metallic: 0.35,
            roughness: 0.35,
            ..Default::default()
        });
        let mapped = material
            .clone()
            .normal_texture(
                MaterialTexture::new(self.image.clone()).sampling(TextureSampling {
                    transform: UvTransform::from_scale_rotation_translation(
                        [self.density; 2],
                        0.,
                        [0.; 2],
                    )
                    .unwrap(),
                    address_u: TextureAddressMode::Repeat,
                    address_v: TextureAddressMode::Repeat,
                    ..Default::default()
                }),
            )
            .normal_scale(self.strength);
        Scene::new()
            .camera(self.controls.camera())
            .color_output(ColorOutput {
                tone_mapping: ToneMapping::Reinhard,
                ..Default::default()
            })
            .light(Light {
                direction: [-0.4, 0.6, 1.],
                color: rgb(0xffffff),
                intensity: 3.,
                ambient: 0.15,
            })
            .object(
                Object::new(Mesh::plane(), material)
                    .position([-0.85, 0., 0.])
                    .scale([1.4, 1.4, 1.]),
            )
            .object(
                Object::new(Mesh::plane(), mapped)
                    .position([0.85, 0., 0.])
                    .scale([if self.mirrored { -1.4 } else { 1.4 }, 1.4, 1.]),
            )
    }
}

impl Render for Normals {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bounds = self.bounds.clone();
        let view_id = cx.entity_id();
        div()
            .size_full()
            .p_6()
            .flex()
            .flex_col()
            .gap_4()
            .bg(rgb(0x0c1421))
            .text_color(rgb(0xe8f0fa))
            .child(div().text_size(px(30.)).child("Surface detail"))
            .child(
                div()
                    .text_color(rgb(0x95aac5))
                    .child("Two flat planes · Right-drag to orbit · Scroll to zoom"),
            )
            .child(
                div().flex().flex_wrap().gap_3().children(
                    [
                        ("less", "Strength −"),
                        ("more", "Strength +"),
                        ("density", "Density"),
                        ("mirror", "Mirror"),
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
                                    "less" => this.strength = (this.strength - 0.25).max(0.),
                                    "more" => this.strength = (this.strength + 0.25).min(3.),
                                    "density" => {
                                        this.density = if this.density < 8. {
                                            this.density * 2.
                                        } else {
                                            1.
                                        }
                                    }
                                    "mirror" => this.mirrored = !this.mirrored,
                                    _ => this
                                        .controls
                                        .set_camera(Camera::orbit(0., 0.15, 4.))
                                        .unwrap(),
                                }
                                cx.notify();
                            }))
                    }),
                ),
            )
            .child(
                div()
                    .id("normal-view")
                    .relative()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .rounded(px(20.))
                    .overflow_hidden()
                    .bg(rgb(0x142237))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                            if this
                                .controls
                                .begin_drag(event.button, event.position, this.bounds.get())
                                .unwrap_or(false)
                            {
                                cx.stop_propagation();
                            }
                        }),
                    )
                    .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                        if this
                            .controls
                            .update_drag(event.position, event.pressed_button, this.bounds.get())
                            .unwrap_or(false)
                        {
                            cx.notify();
                        }
                        if this.controls.is_dragging() {
                            cx.stop_propagation();
                        }
                    }))
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
                    .on_scroll_wheel(cx.listener(|this, event: &gpui::ScrollWheelEvent, _, cx| {
                        if this
                            .controls
                            .scroll(f32::from(event.delta.pixel_delta(px(20.)).y))
                            .unwrap_or(false)
                        {
                            cx.stop_propagation();
                            cx.notify();
                        }
                    }))
                    .child(viewport3d("scene", self.scene()).size_full())
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
                    ),
            )
            .child(
                div()
                    .flex()
                    .justify_around()
                    .child("Smooth")
                    .child("Normal map"),
            )
            .child(div().text_color(rgb(0xa8bdd6)).child(format!(
                "Strength {:.2} · Density {:.0}× · Mirrored {}",
                self.strength, self.density, self.mirrored,
            )))
    }
}

fn main() {
    application().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1200.), px(760.)),
                    cx,
                ))),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| Normals::new(window, cx)),
        )
        .expect("failed to open normal mapping example");
    });
}
