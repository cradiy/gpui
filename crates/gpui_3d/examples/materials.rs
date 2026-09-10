use gpui::{
    App, Bounds, Context, MouseButton, Pixels, Render, Window, WindowBounds, WindowOptions, canvas,
    div, prelude::*, px, rgb, size,
};
use gpui_3d::{
    AlphaMode, Camera, ColorOutput, Light, Material, MaterialTexture, Mesh, Object,
    OrbitController, PbrMaterial, Projection, Scene, SphereOptions, TextureAddressMode,
    TextureFilter, TextureMipFilter, TextureSampling, ToneMapping, UvTransform, viewport3d,
};
use gpui_platform::application;
use std::{cell::Cell, rc::Rc, sync::Arc};

struct Materials {
    mesh: Mesh,
    controls: OrbitController,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    roughness: f32,
    emission: f32,
    maps: bool,
    normal: bool,
    ao: bool,
    exposure: f32,
    tone_mapping: ToneMapping,
    address: TextureAddressMode,
    filter: TextureFilter,
    mip_filter: TextureMipFilter,
    max_anisotropy: u16,
    alpha: AlphaMode,
    normal_image: Arc<gpui::RenderImage>,
    pattern: Arc<gpui::RenderImage>,
    density: f32,
    emission_offset: f32,
    metallic_roughness: Arc<gpui::RenderImage>,
    emissive: Arc<gpui::RenderImage>,
    _activation: gpui::Subscription,
}

impl Materials {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (mesh, _) = Mesh::sphere(SphereOptions {
            radius: 0.7,
            segments: [96, 48],
        })
        .unwrap()
        .generate_tangents()
        .unwrap()
        .into_parts();
        Self {
            mesh,
            controls: OrbitController::new(Camera::orbit(0., 0.12, 7.5)).unwrap(),
            bounds: Rc::new(Cell::new(Bounds::default())),
            roughness: 0.4,
            emission: 1.,
            maps: true,
            normal: false,
            ao: false,
            exposure: 0.,
            tone_mapping: ToneMapping::Reinhard,
            address: TextureAddressMode::Repeat,
            filter: TextureFilter::Linear,
            mip_filter: TextureMipFilter::Linear,
            max_anisotropy: 4,
            alpha: AlphaMode::Mask,
            normal_image: Arc::new(gpui::RenderImage::new(vec![image::Frame::new(
                image::RgbaImage::from_fn(128, 128, |x, y| {
                    let u = x as f32 / 128. * std::f32::consts::TAU;
                    let v = y as f32 / 128. * std::f32::consts::TAU;
                    let n = [-0.7 * u.cos() * v.sin(), -0.7 * u.sin() * v.cos(), 1.];
                    let length = n.iter().map(|v| v * v).sum::<f32>().sqrt();
                    let n = n.map(|v| ((v / length * 0.5 + 0.5) * 255.).round() as u8);
                    image::Rgba([n[0], n[1], n[2], 255])
                }),
            )])),
            pattern: Arc::new(gpui::RenderImage::new(vec![image::Frame::new(
                image::RgbaImage::from_fn(64, 64, |x, y| {
                    let alpha = match (x / 16 + y / 16) % 3 {
                        0 => 0,
                        1 => 120,
                        _ => 255,
                    };
                    image::Rgba([100, 220, 230, alpha])
                }),
            )])),
            density: 1.,
            emission_offset: 0.,
            metallic_roughness: Arc::new(gpui::RenderImage::new(vec![image::Frame::new(
                image::RgbaImage::from_fn(128, 128, |x, y| {
                    let roughness = if (x / 32 + y / 32) % 2 == 0 { 64 } else { 255 };
                    let metallic = if x < 64 { 255 } else { 0 };
                    image::Rgba([roughness, roughness, metallic, 255])
                }),
            )])),
            emissive: Arc::new(gpui::RenderImage::new(vec![image::Frame::new(
                image::RgbaImage::from_fn(128, 128, |x, y| {
                    let lit = x % 32 < 4 || y % 32 < 4;
                    image::Rgba(if lit {
                        [110, 220, 255, 255]
                    } else {
                        [0, 0, 0, 255]
                    })
                }),
            )])),
            _activation: cx.observe_window_activation(window, |this, window, _| {
                if !window.is_window_active() {
                    this.controls.cancel_drag();
                }
            }),
        }
    }

    fn scene(&self) -> Scene {
        let mut scene = Scene::new()
            .camera(self.controls.camera())
            .color_output(ColorOutput {
                exposure: self.exposure,
                tone_mapping: self.tone_mapping,
            })
            .light(Light {
                direction: [-0.4, 0.6, 1.],
                color: rgb(0xffffff),
                intensity: 5.,
                ambient: 0.12,
            });
        for (x, name, color, metallic, emissive) in [
            (-1.7, "Dielectric", 0xdca773, 0., [0.; 3]),
            (0., "Metal", 0xdca773, 1., [0.; 3]),
            (
                1.7,
                "Emission",
                0x325a68,
                0.,
                [0.04 * self.emission, 0.5 * self.emission, self.emission],
            ),
        ] {
            let mut material = Material::color(rgb(color)).pbr(PbrMaterial {
                metallic,
                roughness: self.roughness,
                emissive,
            });
            let sampling = TextureSampling {
                mag_filter: None,
                transform: UvTransform::from_scale_rotation_translation(
                    [self.density; 2],
                    0.,
                    [0.; 2],
                )
                .unwrap(),
                address_u: self.address,
                address_v: self.address,
                filter: self.filter,
                mip_filter: self.mip_filter,
                max_anisotropy: self.max_anisotropy,
            };
            if self.normal {
                material = material.normal_texture(
                    MaterialTexture::new(self.normal_image.clone()).sampling(sampling),
                );
            }
            if self.ao {
                material = material.occlusion_texture(
                    MaterialTexture::new(self.metallic_roughness.clone()).sampling(sampling),
                );
            }
            if self.maps {
                material = material
                    .metallic_roughness_texture(
                        MaterialTexture::new(self.metallic_roughness.clone()).sampling(sampling),
                    )
                    .emissive_texture(
                        MaterialTexture::new(self.emissive.clone()).sampling(TextureSampling {
                            transform: UvTransform::from_scale_rotation_translation(
                                [self.density; 2],
                                0.,
                                [self.emission_offset, 0.],
                            )
                            .unwrap(),
                            ..sampling
                        }),
                    );
            }
            scene = scene.object(
                Object::new(self.mesh.clone(), material)
                    .position([x, 0., 0.])
                    .id(name),
            );
        }
        let sampling = TextureSampling {
            mag_filter: None,
            transform: UvTransform::from_scale_rotation_translation(
                [self.density; 2],
                0.2,
                [self.emission_offset; 2],
            )
            .unwrap(),
            address_u: self.address,
            address_v: self.address,
            filter: self.filter,
            mip_filter: self.mip_filter,
            max_anisotropy: self.max_anisotropy,
        };
        scene
            .object(
                Object::new(Mesh::plane(), Material::color(rgb(0xe1b27d)).unlit(true))
                    .position([0., -1.3, -0.1])
                    .scale([4.6, 0.7, 1.]),
            )
            .object(
                Object::new(
                    Mesh::plane(),
                    Material::image(self.pattern.clone())
                        .image_sampling(sampling)
                        .alpha_mode(self.alpha)
                        .unlit(true),
                )
                .position([0., -1.3, 0.])
                .scale([4.6, 0.7, 1.])
                .id("Alpha / UV"),
            )
    }
}

impl Render for Materials {
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
            .child(div().text_size(px(30.)).child("Surface and light"))
            .child(
                div().text_color(rgb(0x95aac5)).child(
                    "Right-drag to orbit · Scroll to zoom · Compare highlight width and color",
                ),
            )
            .child(
                div().flex().flex_wrap().gap_3().children(
                    [
                        ("smooth", "Roughness −"),
                        ("rough", "Roughness +"),
                        ("dim", "Emission −"),
                        ("glow", "Emission +"),
                        ("maps", "Toggle maps"),
                        ("density", "Map density"),
                        ("shift", "Shift emission"),
                        ("normal", "Normal map"),
                        ("ao", "Occlusion"),
                        ("alpha", "Alpha mode"),
                        ("address", "UV address"),
                        ("filter", "Texture filter"),
                        ("mips", "Mip filter"),
                        ("anisotropy", "Anisotropy"),
                        ("exposure", "Exposure"),
                        ("tone", "Tone mapping"),
                        ("projection", "Switch projection"),
                        ("focal", "Focal length"),
                        ("lens", "Lens shift"),
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
                                    "smooth" => this.roughness = (this.roughness - 0.1).max(0.),
                                    "rough" => this.roughness = (this.roughness + 0.1).min(1.),
                                    "dim" => this.emission = (this.emission - 0.5).max(0.),
                                    "glow" => this.emission = (this.emission + 0.5).min(8.),
                                    "maps" => this.maps = !this.maps,
                                    "density" => this.density = if this.density < 4. { this.density * 2. } else { 1. },
                                    "shift" => this.emission_offset = (this.emission_offset + 0.0625) % 1.,
                                    "normal" => this.normal = !this.normal,
                                    "ao" => this.ao = !this.ao,
                                    "alpha" => this.alpha = match this.alpha { AlphaMode::Opaque => AlphaMode::Mask, AlphaMode::Mask => AlphaMode::Blend, AlphaMode::Blend => AlphaMode::Opaque },
                                    "address" => this.address = match this.address { TextureAddressMode::Clamp => TextureAddressMode::Repeat, TextureAddressMode::Repeat => TextureAddressMode::Mirror, TextureAddressMode::Mirror => TextureAddressMode::Clamp },
                                    "filter" => {
                                        this.filter = if this.filter == TextureFilter::Linear { TextureFilter::Nearest } else { TextureFilter::Linear };
                                        if this.filter == TextureFilter::Nearest { this.max_anisotropy = 1; }
                                    },
                                    "mips" => {
                                        this.mip_filter = match this.mip_filter { TextureMipFilter::None => TextureMipFilter::Nearest, TextureMipFilter::Nearest => TextureMipFilter::Linear, TextureMipFilter::Linear => TextureMipFilter::None };
                                        if this.mip_filter != TextureMipFilter::Linear { this.max_anisotropy = 1; }
                                    },
                                    "anisotropy" => {
                                        this.max_anisotropy = if this.max_anisotropy < 16 { this.max_anisotropy * 2 } else { 1 };
                                        if this.max_anisotropy > 1 { this.filter = TextureFilter::Linear; this.mip_filter = TextureMipFilter::Linear; }
                                    },
                                    "exposure" => this.exposure = if this.exposure < 2. { this.exposure+1. } else { -2. },
                                    "tone" => this.tone_mapping = if this.tone_mapping == ToneMapping::Reinhard { ToneMapping::None } else { ToneMapping::Reinhard },
                                    "focal" => {
                                        let mut camera = this.controls.camera();
                                        let focal = camera.projection.focal_length(24.).unwrap_or(0.);
                                        camera.projection = Projection::from_focal_length(
                                            if focal < 34. { 35. } else if focal < 49. { 50. } else if focal < 84. { 85. } else { 35. }, 24.,
                                        ).unwrap();
                                        this.controls.set_camera(camera).unwrap();
                                    }
                                    "lens" => {
                                        let mut camera = this.controls.camera();
                                        camera.lens_shift = if camera.lens_shift[0] == 0. { [0.6,0.25] } else if camera.lens_shift[0] > 0. { [-0.6,0.25] } else { [0.;2] };
                                        this.controls.set_camera(camera).unwrap();
                                    }
                                    "projection" => {
                                        let mut camera = this.controls.camera();
                                        let distance = camera
                                            .eye
                                            .iter()
                                            .zip(camera.target)
                                            .map(|(a, b)| (a - b).powi(2))
                                            .sum::<f32>()
                                            .sqrt();
                                        camera.projection = match camera.projection {
                                            Projection::Perspective { vertical_fov } => {
                                                Projection::Orthographic {
                                                    vertical_size: 2.
                                                        * distance
                                                        * (vertical_fov * 0.5).tan(),
                                                }
                                            }
                                            Projection::Orthographic { vertical_size } => {
                                                Projection::Perspective {
                                                    vertical_fov: 2.
                                                        * (vertical_size / (2. * distance)).atan(),
                                                }
                                            }
                                        };
                                        this.controls.set_camera(camera).unwrap();
                                    }
                                    _ => {
                                        this.controls
                                            .set_camera(Camera::orbit(0., 0.12, 7.5))
                                            .unwrap();
                                    }
                                }
                                cx.notify();
                            }))
                    }),
                ),
            )
            .child(
                div()
                    .id("materials")
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
                    .justify_between()
                    .children(["Dielectric", "Metal", "Emission"]),
            )
            .child(div().text_color(rgb(0xa8bdd6)).child(format!("Normal {} · AO {} · {:?} · {:?} / {:?} · Exposure {:+.0} · {:?}", self.normal, self.ao, self.alpha, self.address, self.filter, self.exposure, self.tone_mapping)))
            .child(div().text_color(rgb(0xa8bdd6)).child(format!("Mip {:?} · Anisotropy {}×", self.mip_filter, self.max_anisotropy)))
            .child(div().text_color(rgb(0xa8bdd6)).child(format!(
                "Roughness {:.2} · Emission {:.1}× · Maps {} · Density {:.0}× · Emission offset {:.3} · {:?} · Lens shift {:?}",
                self.roughness,
                self.emission,
                if self.maps { "On" } else { "Off" },
                self.density,
                self.emission_offset,
                self.controls.camera().projection,
                self.controls.camera().lens_shift
            )))
    }
}

fn main() {
    application().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1200.), px(900.)),
                    cx,
                ))),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| Materials::new(window, cx)),
        )
        .expect("failed to open materials example");
    });
}
