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

#[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
#[path = "materials/programs.rs"]
mod programs;

struct Materials {
    custom: bool,
    bands: f32,
    sphere_brightness: f32,
    outline_mode: u8,
    outline_weighted: bool,
    program_error: Option<gpui::SharedString>,
    #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
    programs: Option<programs::Programs>,
    mesh: Mesh,
    colored_mesh: Mesh,
    colored_strip: Mesh,
    vertex_colors: bool,
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
        let colored_mesh = mesh
            .with_vertex_colors(
                mesh.vertices()
                    .iter()
                    .map(|v| {
                        let rgb = v.normal.map(|n| (n * 0.5 + 0.5).clamp(0., 1.));
                        [rgb[0], rgb[1], rgb[2], 1.]
                    })
                    .collect(),
            )
            .unwrap();
        let strip = Mesh::plane();
        let colored_strip = strip
            .with_vertex_colors(
                strip
                    .vertices()
                    .iter()
                    .map(|v| [v.uv[0], 1. - v.uv[0], 0.75, v.uv[0]])
                    .collect(),
            )
            .unwrap();
        Self {
            custom: false,
            bands: 3.,
            sphere_brightness: 1.5,
            outline_mode: 2,
            outline_weighted: false,
            program_error: None,
            #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
            programs: None,
            mesh,
            colored_mesh,
            colored_strip,
            vertex_colors: false,
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
        if self.custom && self.program_error.is_some() {
            return Scene::new();
        }
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
            if self.custom {
                material = Material::color(rgb(0x7ac9dc)).pbr(PbrMaterial {
                    metallic: 0.,
                    roughness: self.roughness,
                    emissive: [0.; 3],
                });
            }
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
            if self.maps && !self.custom {
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
            #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
            if self.custom
                && let Some(programs) = &self.programs
            {
                material = match name {
                    "Metal" => material.program(programs.toon.clone()),
                    "Emission" => material.program(programs.sphere.clone()),
                    _ => material,
                };
                if self.outline_mode != 0 {
                    use gpui_3d::{
                        MeshPass, MeshPassCull, MeshPassExpansion, MeshPassSpace, MeshPassState,
                    };
                    let mut expansion = if self.outline_mode == 1 {
                        MeshPassExpansion::new(MeshPassSpace::World, 0.045)
                    } else {
                        MeshPassExpansion::new(MeshPassSpace::Pixels, 4.)
                    };
                    if self.outline_weighted {
                        expansion = expansion.weight("width", 1.);
                    }
                    material = material.mesh_passes([MeshPass::new(programs.outline.clone())
                        .state(MeshPassState {
                            cull: MeshPassCull::Front,
                            ..Default::default()
                        })
                        .expansion(expansion)]);
                }
            }
            scene = scene.object(
                Object::new(
                    if self.vertex_colors {
                        self.colored_mesh.clone()
                    } else {
                        self.mesh.clone()
                    },
                    material,
                )
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
                    if self.vertex_colors {
                        self.colored_strip.clone()
                    } else {
                        Mesh::plane()
                    },
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
        if self.custom
            && self.program_error.is_none()
            && self
                .programs
                .as_ref()
                .is_none_or(|programs| !programs.matches_window(_window))
        {
            self.programs = None;
            match programs::Programs::new(_window, self.bands, self.sphere_brightness, &self.mesh) {
                Ok(programs) => self.programs = Some(programs),
                Err(error) => self.program_error = Some(format!("{error:#}").into()),
            }
        }
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
                        ("vertex-colors", "Vertex colors"),
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
                    .chain(cfg!(all(feature = "wgpu", not(target_family = "wasm"))).then_some(("programs", "PBR / Custom")))
                    .chain(self.custom.then_some(("bands", "Toon bands")))
                    .chain(self.custom.then_some(("sphere", "Sphere brightness")))
                    .chain(self.custom.then_some(("outline", match self.outline_mode { 0 => "Outline: off", 1 => "Outline: world", _ => "Outline: pixels" })))
                    .chain(self.custom.then_some(("outline-weights", if self.outline_weighted { "Width: weighted" } else { "Width: uniform" })))
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
                                    "outline" => this.outline_mode = (this.outline_mode + 1) % 3,
                                    "outline-weights" => this.outline_weighted = !this.outline_weighted,
                                    #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
                                    "programs" => {
                                        this.custom = !this.custom;
                                        this.program_error = None;
                                    }
                                    #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
                                    "bands" | "sphere" => {
                                        let bands = if id == "bands" { if this.bands < 6. { this.bands + 1. } else { 2. } } else { this.bands };
                                        let brightness = if id == "sphere" { if this.sphere_brightness < 3. { this.sphere_brightness + 0.5 } else { 0.5 } } else { this.sphere_brightness };
                                        if let Some(programs) = &mut this.programs {
                                            match programs.update(bands, brightness) {
                                                Ok(()) => { this.bands = bands; this.sphere_brightness = brightness; }
                                                Err(error) => this.program_error = Some(format!("{error:#}").into()),
                                            }
                                        }
                                    }
                                    "smooth" => this.roughness = (this.roughness - 0.1).max(0.),
                                    "rough" => this.roughness = (this.roughness + 0.1).min(1.),
                                    "dim" => this.emission = (this.emission - 0.5).max(0.),
                                    "glow" => this.emission = (this.emission + 0.5).min(8.),
                                    "maps" => this.maps = !this.maps,
                                    "vertex-colors" => this.vertex_colors = !this.vertex_colors,
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
            .when(self.custom, |view| view.child(div().text_color(rgb(0xa8bdd6))
                .child(format!("Toon: {:.0} bands · Sphere map: {:.1}× · Camera-space reflection", self.bands, self.sphere_brightness))))
            .when_some(self.program_error.clone(), |view, error| view.child(
                div().text_color(rgb(0xffa080)).child(error)))
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
                    .children(if self.custom { ["PBR", "Toon", "Sphere Map"] } else { ["Dielectric", "Metal", "Emission"] }),
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
