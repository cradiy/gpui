use gpui::{
    App, Bounds, Context, MouseButton, Pixels, Render, Window, WindowBounds, WindowOptions, canvas,
    div, prelude::*, px, rgb, size,
};
use gpui_3d::{
    AffineTransform, Camera, EvaluatedScene, Interpolation, Keyframe, Material, Mesh, MorphTarget,
    MorphTargets, Node, NodeHandle, OrbitController, Projection, PunctualLight, RotationTrack,
    SceneGraph, Skin, SkinInfluence, SubtreeInstance, TransformTrack, VectorTrack, viewport3d,
};
use gpui_platform::application;
use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant},
};

const ANIMATION_LENGTH: Duration = Duration::from_secs(4);

#[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
#[path = "scene/gpu.rs"]
mod gpu;

fn deformation_amount(position: Duration) -> f32 {
    if position == ANIMATION_LENGTH {
        0.
    } else {
        (std::f32::consts::PI * position.as_secs_f32() / ANIMATION_LENGTH.as_secs_f32()).sin()
    }
}

fn animation(interpolation: Interpolation) -> TransformTrack {
    let vector = |values: [[f32; 3]; 3]| {
        VectorTrack::new(
            values
                .into_iter()
                .enumerate()
                .map(|(index, value)| Keyframe::new(Duration::from_secs(index as u64 * 2), value)),
            interpolation,
        )
        .unwrap()
    };
    TransformTrack::default()
        .translation(vector([[0.; 3], [0., 1., 0.], [0.; 3]]))
        .scale(vector([[1.; 3], [1.2, 0.8, 1.2], [1.; 3]]))
        .rotation(
            RotationTrack::new(
                [
                    Keyframe::new(Duration::ZERO, [0., 0., 0., 1.]),
                    Keyframe::new(Duration::from_secs(2), [0., (3_f32).sqrt() * 0.5, 0., 0.5]),
                    Keyframe::new(ANIMATION_LENGTH, [0., 0., 0., 1.]),
                ],
                interpolation,
            )
            .unwrap(),
        )
}

fn local(position: [f32; 3], scale: [f32; 3]) -> AffineTransform {
    AffineTransform::from_trs(position, [0., 0., 0., 1.], scale).unwrap()
}

fn taper(mesh: &Mesh, amount: f32) -> Mesh {
    if amount == 0. {
        return mesh.clone();
    }
    let vertices = mesh
        .vertices()
        .iter()
        .map(|vertex| {
            let [x, y, z] = vertex.position;
            let [nx, ny, nz] = vertex.normal;
            let scale = 1. + amount * y;
            gpui_3d::Vertex {
                position: [x * scale, y, z * scale],
                normal: [
                    nx / scale,
                    ny - amount * (x * nx + z * nz) / scale,
                    nz / scale,
                ],
                uv: vertex.uv,
            }
        })
        .collect();
    let tangents = mesh
        .tangents()
        .unwrap()
        .iter()
        .zip(mesh.vertices())
        .map(|(t, v)| {
            let scale = 1. + amount * v.position[1];
            [
                scale * t[0] + amount * v.position[0] * t[1],
                t[1],
                scale * t[2] + amount * v.position[2] * t[1],
                t[3],
            ]
        })
        .collect();
    mesh.with_vertices(vertices, Some(tangents)).unwrap()
}

fn morphs(mesh: &Mesh) -> MorphTargets {
    let shear = mesh
        .with_vertices(
            mesh.vertices()
                .iter()
                .map(|v| gpui_3d::Vertex {
                    position: [
                        v.position[0] + 0.85 * v.position[1],
                        v.position[1],
                        v.position[2],
                    ],
                    normal: [v.normal[0], v.normal[1] - 0.85 * v.normal[0], v.normal[2]],
                    uv: v.uv,
                })
                .collect(),
            Some(
                mesh.tangents()
                    .unwrap()
                    .iter()
                    .map(|t| [t[0] + 0.85 * t[1], t[1], t[2], t[3]])
                    .collect(),
            ),
        )
        .unwrap();
    let targets = [taper(mesh, 1.2), shear].map(|shape| MorphTarget {
        positions: Some(
            shape
                .vertices()
                .iter()
                .zip(mesh.vertices())
                .map(|(a, b)| std::array::from_fn(|i| a.position[i] - b.position[i]))
                .collect(),
        ),
        normals: Some(
            shape
                .vertices()
                .iter()
                .zip(mesh.vertices())
                .map(|(a, b)| {
                    let length = a.normal.iter().map(|v| v * v).sum::<f32>().sqrt();
                    std::array::from_fn(|i| a.normal[i] / length - b.normal[i])
                })
                .collect(),
        ),
        tangents: Some(
            shape
                .tangents()
                .unwrap()
                .iter()
                .zip(mesh.tangents().unwrap())
                .map(|(a, b)| std::array::from_fn(|i| a[i] - b[i]))
                .collect(),
        ),
    });
    MorphTargets::new(mesh.clone(), targets).unwrap()
}

fn body_geometry() -> Mesh {
    let cube = Mesh::cube();
    let mut vertices = Vec::new();
    let mut tangents = Vec::new();
    let mut indices = Vec::new();
    const STEPS: u32 = 8;
    for face in 0..6 {
        let corners = &cube.vertices()[face * 4..face * 4 + 4];
        let start = vertices.len() as u32;
        for y in 0..=STEPS {
            for x in 0..=STEPS {
                let u = x as f32 / STEPS as f32;
                let v = y as f32 / STEPS as f32;
                vertices.push(gpui_3d::Vertex {
                    position: std::array::from_fn(|i| {
                        corners[0].position[i]
                            + u * (corners[1].position[i] - corners[0].position[i])
                            + v * (corners[3].position[i] - corners[0].position[i])
                    }),
                    normal: corners[0].normal,
                    uv: [u, 1. - v],
                });
                tangents.push(cube.tangents().unwrap()[face * 4]);
                if x < STEPS && y < STEPS {
                    let a = start + y * (STEPS + 1) + x;
                    let b = a + 1;
                    let d = a + STEPS + 1;
                    let c = d + 1;
                    indices.extend([a, b, c, a, c, d]);
                }
            }
        }
    }
    Mesh::new(vertices, indices)
        .with_tangents(tangents)
        .unwrap()
}

struct SceneDemo {
    graph: SceneGraph,
    evaluated: EvaluatedScene,
    instances: Vec<SubtreeInstance>,
    body: NodeHandle,
    camera: NodeHandle,
    rig_camera: bool,
    resolution: usize,
    color_samples: u32,
    rig_lights: bool,
    body_mesh: Mesh,
    morphs: MorphTargets,
    skin: Skin,
    skinning: bool,
    gpu_enabled: bool,
    #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
    gpu: Option<gpu::Deformation>,
    deformation: usize,
    morph_weights: [f32; 2],
    mesh_sample: (Duration, usize, [f32; 2], bool, bool),
    selected: usize,
    hovered: Option<usize>,
    raised: [bool; 3],
    tinted: [bool; 3],
    hidden: [bool; 3],
    controls: OrbitController,
    camera_frame: Instant,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    tracks: [TransformTrack; 3],
    position: Duration,
    playing: bool,
    last_frame: Instant,
    _activation: gpui::Subscription,
}
impl SceneDemo {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let geometry = body_geometry();
        let mut source = SceneGraph::new();
        let root = source.insert(None, Node::new().id("assembly")).unwrap();
        let camera_node = source
            .insert(
                Some(root),
                Node::new()
                    .id("camera")
                    .transform(local([0., 1.5, 4.], [1.; 3]))
                    .camera(Camera {
                        eye: [0.; 3],
                        target: [0., -1.5, -4.],
                        ..Default::default()
                    }),
            )
            .unwrap();
        let body = source
            .insert(
                Some(root),
                Node::new()
                    .id("body")
                    .mesh(geometry.clone(), Material::color(rgb(0x8dd8e8)))
                    .transform(local([0., -0.2, 0.], [1.3, 0.9, 1.])),
            )
            .unwrap();
        source
            .insert(
                Some(root),
                Node::new()
                    .id("cap")
                    .mesh(geometry.clone(), Material::color(rgb(0xf4cf89)))
                    .transform(local([0.4, 0.55, 0.], [0.5; 3])),
            )
            .unwrap();
        source
            .insert(
                Some(root),
                Node::new()
                    .id("base")
                    .mesh(geometry.clone(), Material::color(rgb(0x526a87)))
                    .transform(local([0., -0.85, 0.], [1.8, 0.12, 1.6])),
            )
            .unwrap();
        let subtree = source.snapshot_subtree(root).unwrap();
        let mut graph = SceneGraph::new();
        let instances = [-2.5, 0., 2.5]
            .into_iter()
            .map(|x| {
                let instance = graph.instantiate(None, &subtree).unwrap();
                graph
                    .set_transform(instance.root(), local([x, 0., 0.], [1.; 3]))
                    .unwrap();
                instance
            })
            .collect();
        let evaluated = graph.evaluate().unwrap();
        let mut camera = Camera::orbit(0.3, 0.35, 10.)
            .frame_bounds(evaluated.bounds().unwrap(), 1.6, 1.3)
            .unwrap();
        camera.near = 0.01;
        camera.far = 100.;
        let morphs = morphs(&geometry);
        let skin = Skin::new(
            [AffineTransform::IDENTITY, local([0., 0.25, 0.], [1.; 3])],
            geometry.vertices().iter().map(|v| {
                let weight = (v.position[1] + 0.5).clamp(0., 1.);
                [
                    SkinInfluence {
                        joint: 0,
                        weight: 1. - weight,
                    },
                    SkinInfluence { joint: 1, weight },
                ]
            }),
        )
        .unwrap();
        Self {
            graph,
            evaluated,
            instances,
            body,
            camera: camera_node,
            rig_camera: false,
            resolution: 1,
            color_samples: 4,
            rig_lights: false,
            body_mesh: geometry,
            morphs,
            skin,
            skinning: false,
            gpu_enabled: false,
            #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
            gpu: None,
            deformation: 0,
            morph_weights: [0.65, 0.35],
            mesh_sample: (Duration::ZERO, 0, [0.65, 0.35], false, false),
            selected: 1,
            hovered: None,
            raised: [false; 3],
            tinted: [false; 3],
            hidden: [false; 3],
            controls: OrbitController::new(camera).unwrap(),
            camera_frame: Instant::now(),
            bounds: Rc::new(Cell::new(Bounds::default())),
            tracks: [
                Interpolation::Step,
                Interpolation::Linear,
                Interpolation::CubicSpline,
            ]
            .map(animation),
            position: Duration::ZERO,
            playing: false,
            last_frame: Instant::now(),
            _activation: cx.observe_window_activation(window, |this, window, _| {
                if !window.is_window_active() {
                    this.controls.cancel_drag();
                }
            }),
        }
    }
    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.evaluate_pose();
        cx.notify();
    }
    fn advance_camera(&mut self, now: Instant) -> bool {
        let changed = self
            .controls
            .advance(now.duration_since(self.camera_frame))
            .unwrap_or(false);
        self.camera_frame = now;
        changed
    }
    fn evaluate_pose(&mut self) {
        let position = if self.deformation != 0 || self.skinning {
            self.position
        } else {
            Duration::ZERO
        };
        let mesh_sample = (
            position,
            self.deformation,
            self.morph_weights,
            self.skinning,
            self.gpu_enabled,
        );
        if self.mesh_sample != mesh_sample {
            let amount = deformation_amount(position);
            let mut mesh = match if self.gpu_enabled {
                0
            } else {
                self.deformation
            } {
                1 => taper(&self.body_mesh, amount * 1.2),
                2 => self
                    .morphs
                    .evaluate(&self.morph_weights.map(|weight| weight * amount))
                    .unwrap(),
                _ => self.body_mesh.clone(),
            };
            if self.skinning && !self.gpu_enabled {
                let half_angle = 0.6 * amount;
                let tip = AffineTransform::from_trs(
                    [0., -0.25, 0.],
                    [0., 0., half_angle.sin(), half_angle.cos()],
                    [1.; 3],
                )
                .unwrap();
                mesh = self
                    .skin
                    .evaluate(&mesh, &[AffineTransform::IDENTITY, tip])
                    .unwrap();
            }
            for instance in &self.instances {
                self.graph
                    .set_mesh(instance.node(self.body).unwrap(), mesh.clone())
                    .unwrap();
            }
            self.mesh_sample = mesh_sample;
        }
        let transforms = self
            .instances
            .iter()
            .zip(&self.tracks)
            .map(|(instance, track)| {
                let root = instance.root();
                let authored = self.graph.node(root).unwrap().local_transform();
                (
                    root,
                    authored
                        .compose(track.sample_transform(self.position).unwrap())
                        .unwrap(),
                )
            });
        self.evaluated = self.graph.evaluate_with_transforms(transforms).unwrap();
    }
    fn button(
        &self,
        id: &'static str,
        label: &'static str,
        active: bool,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .px_4()
            .py_2()
            .rounded(px(10.))
            .cursor_pointer()
            .bg(rgb(if active { 0x406b88 } else { 0x253a53 }))
            .hover(|style| style.bg(rgb(0x42627e)))
            .child(label)
    }
}
impl Render for SceneDemo {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
        self.advance_camera(now);
        if self.controls.is_animating() {
            window.request_animation_frame();
        }
        if self.playing {
            self.position =
                (self.position + now.duration_since(self.last_frame)).min(ANIMATION_LENGTH);
            self.evaluate_pose();
            self.playing = self.position < ANIMATION_LENGTH;
            if self.playing {
                window.request_animation_frame();
            }
        }
        self.last_frame = now;
        let scene = if self.rig_camera {
            self.evaluated
                .scene_from_camera(self.instances[self.selected].node(self.camera).unwrap())
                .unwrap()
        } else {
            self.evaluated.scene(self.controls.camera())
        };
        #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
        let viewport: anyhow::Result<gpui_3d::Viewport3d> = if self.gpu_enabled {
            (|| {
                if self
                    .gpu
                    .as_ref()
                    .is_none_or(|gpu| !gpu.matches_window(window))
                {
                    self.gpu = Some(gpu::Deformation::new(
                        window,
                        self.morphs.clone(),
                        self.skin.clone(),
                    )?);
                }
                let amount = deformation_amount(self.position);
                let sample = gpu::Sample {
                    weights: if self.deformation == 2 {
                        self.morph_weights.map(|weight| weight * amount)
                    } else {
                        [0.; 2]
                    },
                    bend: self.skinning.then_some(0.6 * amount),
                };
                let bodies: Vec<_> = self
                    .instances
                    .iter()
                    .map(|instance| instance.node(self.body).unwrap())
                    .collect();
                let gpu = self.gpu.as_mut().unwrap();
                let viewport = gpu.view(scene, &bodies, sample)?;
                if gpu.is_pending() {
                    window.request_animation_frame();
                }
                Ok(viewport)
            })()
        } else {
            Ok(viewport3d("scene", scene))
        };
        #[cfg(not(all(feature = "wgpu", not(target_family = "wasm"))))]
        let viewport: anyhow::Result<gpui_3d::Viewport3d> = Ok(viewport3d("scene", scene));
        let viewport = match viewport {
            Ok(viewport) => viewport
                .resolution_scale([0.5, 1., 2.][self.resolution])
                .color_samples(self.color_samples)
                .size_full()
                .on_object_hover(cx.listener(|this, hit: &Option<gpui_3d::Hit>, _, cx| {
                    let hovered = hit.as_ref().and_then(|hit| {
                        this.instances.iter().position(|instance| {
                            instance.mappings().any(|(_, node)| Some(node) == hit.node)
                        })
                    });
                    if this.hovered != hovered {
                        this.hovered = hovered;
                        cx.notify();
                    }
                }))
                .on_object_click(cx.listener(|this, hit: &gpui_3d::Hit, _, cx| {
                    if let Some(selected) = this.instances.iter().position(|instance| {
                        instance.mappings().any(|(_, node)| Some(node) == hit.node)
                    }) {
                        this.selected = selected;
                        cx.notify();
                    }
                }))
                .into_any_element(),
            Err(error) => {
                self.playing = false;
                div()
                    .p_6()
                    .text_color(rgb(0xf09e8e))
                    .child(format!("GPU deformation unavailable: {error}"))
                    .into_any_element()
            }
        };
        let bounds = self.bounds.clone();
        let view_id = cx.entity_id();
        let mut stage = div()
            .id("stage")
            .relative()
            .w_full()
            .flex_1()
            .min_h_0()
            .rounded(px(24.))
            .overflow_hidden()
            .bg(rgb(0x142237));
        for button in [MouseButton::Right, MouseButton::Middle] {
            stage = stage
                .on_mouse_down(
                    button,
                    cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                        if this.rig_camera {
                            return;
                        }
                        if this.advance_camera(Instant::now()) {
                            cx.notify();
                        }
                        if this
                            .controls
                            .begin_drag(button, event.position, this.bounds.get())
                            .unwrap_or(false)
                        {
                            cx.stop_propagation();
                        }
                    }),
                )
                .on_mouse_up(
                    button,
                    cx.listener(move |this, _, _, cx| {
                        if this.controls.end_drag(button) {
                            cx.stop_propagation();
                        }
                    }),
                )
                .on_mouse_up_out(
                    button,
                    cx.listener(move |this, _, _, _| {
                        this.controls.end_drag(button);
                    }),
                );
        }
        stage = stage
            .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                if this.rig_camera {
                    return;
                }
                if this.advance_camera(Instant::now()) {
                    cx.notify();
                }
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
            .on_hover(cx.listener(|this, hovered: &bool, _, _| {
                if !hovered {
                    this.controls.cancel_drag();
                }
            }))
            .on_scroll_wheel(cx.listener(|this, event: &gpui::ScrollWheelEvent, _, cx| {
                if this.rig_camera {
                    return;
                }
                if this.advance_camera(Instant::now()) {
                    cx.notify();
                }
                if this
                    .controls
                    .scroll(f32::from(event.delta.pixel_delta(px(20.)).y))
                    .unwrap_or(false)
                {
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .child(viewport)
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
        div().size_full().p_6().flex().flex_col().gap_4().bg(rgb(0x0b1422)).text_color(rgb(0xeaf2fc))
            .child(div().text_size(px(30.)).child("Shared shapes, independent nodes"))
            .child(div().text_color(rgb(0x9eb1cb)).child(if self.gpu_enabled {
                "GPU deformation · Select 1 / 2 / 3 with the buttons · Orbit controls remain available outside Rig camera"
            } else if self.rig_camera {
                "Camera follows the selected assembly · Select 1 / 2 / 3 to switch · Disable Rig camera for orbit controls"
            } else {
                "Click an assembly to select · Right-drag to orbit · Middle-drag to pan · Scroll to zoom"
            }))
            .child(div().flex().flex_wrap().gap_3()
                .children([("one", "1 · Step"), ("two", "2 · Linear"), ("three", "3 · Cubic")].into_iter().enumerate().map(|(index, (id, label))| {
                    self.button(id, label, self.selected == index || self.hovered == Some(index)).on_click(cx.listener(move |this, _, _, cx| { this.selected = index; cx.notify(); }))
                }))
                .child(self.button("move", "Move body", self.raised[self.selected]).on_click(cx.listener(|this, _, _, cx| {
                    let index = this.selected;
                    this.raised[index] = !this.raised[index];
                    this.graph.set_transform(this.instances[index].node(this.body).unwrap(),
                        local([0., if this.raised[index] { 0.8 } else { -0.2 }, 0.], [1.3, 0.9, 1.])).unwrap();
                    this.refresh(cx);
                })))
                .child(self.button("tint", "Tint body", self.tinted[self.selected]).on_click(cx.listener(|this, _, _, cx| {
                    let index = this.selected;
                    this.tinted[index] = !this.tinted[index];
                    this.graph.set_material(this.instances[index].node(this.body).unwrap(),
                        Material::color(rgb(if this.tinted[index] { 0xf09e8e } else { 0x8dd8e8 }))).unwrap();
                    this.refresh(cx);
                })))
                .child(self.button("hide", "Hide / show", self.hidden[self.selected]).on_click(cx.listener(|this, _, _, cx| {
                    let index = this.selected;
                    this.hidden[index] = !this.hidden[index];
                    this.graph.set_visible(this.instances[index].root(), !this.hidden[index]).unwrap();
                    this.refresh(cx);
                })))
                .child(self.button("projection", "Projection", false).on_click(cx.listener(|this, _, _, cx| {
                    let handle = this.instances[this.selected].node(this.camera).unwrap();
                    let mut camera = if this.rig_camera { this.graph.node(handle).unwrap().local_camera().unwrap() } else { this.controls.camera() };
                    let distance = camera.eye.iter().zip(camera.target).map(|(a,b)| (a-b).powi(2)).sum::<f32>().sqrt();
                    camera.projection = match camera.projection {
                        Projection::Perspective { vertical_fov } => Projection::Orthographic { vertical_size: 2. * distance * (vertical_fov * 0.5).tan() },
                        Projection::Orthographic { vertical_size } => Projection::Perspective { vertical_fov: 2. * (vertical_size / (2. * distance)).atan() },
                    };
                    if this.rig_camera { this.graph.set_camera(handle, Some(camera)).unwrap(); this.refresh(cx); }
                    else { this.controls.set_camera(camera).unwrap(); cx.notify(); }
                })))
                .child(self.button("rig-camera", "Rig camera", self.rig_camera).on_click(cx.listener(|this, _, _, cx| {
                    this.rig_camera = !this.rig_camera;
                    this.controls.cancel_drag(); cx.notify();
                })))
                .child(self.button("damping", "Camera damping", self.controls.damping().is_some()).on_click(cx.listener(|this, _, _, cx| {
                    let half_life = this.controls.damping().is_none().then(|| Duration::from_millis(80));
                    this.controls.set_damping(half_life).unwrap();
                    this.camera_frame = Instant::now();
                    cx.notify();
                })))
                .child(self.button("rig-lights", "Rig lights", self.rig_lights).on_click(cx.listener(|this, _, _, cx| {
                    this.rig_lights = !this.rig_lights;
                    for instance in &this.instances {
                        this.graph.set_light(instance.node(this.camera).unwrap(), this.rig_lights.then(||
                            PunctualLight::spot([0.; 3], [0., -1.5, -4.]).intensity(30.).range(Some(12.)).cone_angles(0.3, 0.8)
                        )).unwrap();
                    }
                    this.refresh(cx);
                })))
                .child(self.button("frame", "Frame selected", false).on_click(cx.listener(|this, _, _, cx| {
                    let rect = this.bounds.get();
                    let aspect = if rect.size.height > px(0.) { rect.size.width / rect.size.height } else { 1.5 };
                    if let Some(bounds) = this.evaluated.node(this.instances[this.selected].root()).and_then(|node| node.subtree_bounds) {
                        this.rig_camera = false;
                        let camera = this.controls.camera().frame_bounds(bounds, aspect, 1.3).unwrap();
                        this.controls.set_camera(camera).unwrap(); cx.notify();
                    }
                })))
                .child(self.button("rotate", "Rotate assembly", false).on_click(cx.listener(|this, _, _, cx| {
                    let root = this.instances[this.selected].root();
                    let local = this.graph.node(root).unwrap().local_transform();
                    let rotation = AffineTransform::from_trs([0.; 3], [0., (0.15_f32).sin(), 0., (0.15_f32).cos()], [1.; 3]).unwrap();
                    this.graph.set_transform(root, local.compose(rotation).unwrap()).unwrap();
                    this.refresh(cx);
                })))
                .child(self.button("reset", "Reset", false).on_click(cx.listener(|this, _, window, cx| { *this = Self::new(window, cx); cx.notify(); }))))
            .child(div().flex().flex_wrap().items_center().gap_3()
                .when(cfg!(all(feature = "wgpu", not(target_family = "wasm"))) && self.deformation != 1, |row| row.child(
                    self.button("gpu", if self.gpu_enabled { "GPU deformation" } else { "CPU deformation" }, self.gpu_enabled)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.gpu_enabled = !this.gpu_enabled;
                            this.hovered = None;
                            this.refresh(cx);
                        }))
                ))
                .child(self.button("deform", "Taper mesh", self.deformation == 1).on_click(cx.listener(|this, _, _, cx| {
                    this.gpu_enabled = false;
                    this.deformation = if this.deformation == 1 { 0 } else { 1 }; this.refresh(cx);
                })))
                .child(self.button("morph", "Blend shapes", self.deformation == 2).on_click(cx.listener(|this, _, _, cx| {
                    this.deformation = if this.deformation == 2 { 0 } else { 2 };
                    if this.deformation == 2 && !this.playing && (this.position == Duration::ZERO || this.position == ANIMATION_LENGTH) {
                        this.position = Duration::from_secs(2);
                    }
                    this.refresh(cx);
                })))
                .child(self.button("skin", "Bend skin", self.skinning).on_click(cx.listener(|this, _, _, cx| {
                    this.skinning = !this.skinning;
                    if this.skinning && !this.playing && (this.position == Duration::ZERO || this.position == ANIMATION_LENGTH) {
                        this.position = Duration::from_secs(2);
                    }
                    this.refresh(cx);
                })))
                .child(self.button("play", if self.playing { "Pause" } else { "Play" }, self.playing).on_click(cx.listener(|this, _, _, cx| {
                    this.playing = !this.playing;
                    if this.playing && this.position == ANIMATION_LENGTH { this.position = Duration::ZERO; }
                    this.last_frame = Instant::now();
                    this.refresh(cx);
                })))
                .child(self.button("seek-back", "−0.25 s", false).on_click(cx.listener(|this, _, _, cx| {
                    this.playing = false;
                    this.position = this.position.saturating_sub(Duration::from_millis(250));
                    this.refresh(cx);
                })))
                .child(self.button("seek-forward", "+0.25 s", false).on_click(cx.listener(|this, _, _, cx| {
                    this.playing = false;
                    this.position = (this.position + Duration::from_millis(250)).min(ANIMATION_LENGTH);
                    this.refresh(cx);
                })))
                .child(self.button("start", "Start pose", false).on_click(cx.listener(|this, _, _, cx| {
                    this.playing = false; this.position = Duration::ZERO; this.refresh(cx);
                })))
                .child(format!("{:.2} / 4.00 s · Translation, rotation and scale", self.position.as_secs_f64())))
            .when(self.deformation == 2, |root| root.child(div().flex().flex_wrap().items_center().gap_3()
                .children([("taper-less", "Taper −", 0, -0.1), ("taper-more", "Taper +", 0, 0.1), ("shear-less", "Shear −", 1, -0.1), ("shear-more", "Shear +", 1, 0.1)]
                    .into_iter().map(|(id, label, index, delta)| self.button(id, label, false).on_click(cx.listener(move |this, _, _, cx| {
                        this.morph_weights[index] = (this.morph_weights[index] + delta).clamp(-0.5, 1.);
                        this.refresh(cx);
                    }))))
                .child(format!("Weights {:.2} / {:.2}", self.morph_weights[0], self.morph_weights[1]))))
            .child(div().flex().flex_wrap().items_center().gap_3()
                .child(self.button("resolution", ["Resolution 0.5×", "Resolution 1×", "Resolution 2×"][self.resolution], self.resolution != 1)
                    .on_click(cx.listener(|this, _, _, cx| { this.resolution = (this.resolution + 1) % 3; cx.notify(); })))
                .child(self.button("samples", if self.color_samples == 4 { "Samples 4×" } else { "Samples 1×" }, self.color_samples == 4)
                    .on_click(cx.listener(|this, _, _, cx| { this.color_samples = if this.color_samples == 4 { 1 } else { 4 }; cx.notify(); })))
                .child(format!("Effective samples: {}", window.scene3d_support().capabilities().map_or(0, |caps| caps.color_samples_for(gpui_3d::ViewportQuality::new([0.5, 1., 2.][self.resolution], self.color_samples))))))
            .child(stage)
            .child(div().text_sm().text_color(rgb(0xa4bad2)).child(format!("Instance {} selected · 3 editable subtrees · Shared mesh topology", self.selected + 1)))
            .when(!window.supports_scene3d(), |root| root.child("3D viewports are unavailable on this renderer."))
    }
}
fn main() {
    application().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1200.), px(850.)),
                    cx,
                ))),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| SceneDemo::new(window, cx)),
        )
        .expect("failed to open scene example");
    });
}
