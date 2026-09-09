use gpui::{
    App, Bounds, Context, MouseButton, Pixels, Render, Window, WindowBounds, WindowOptions, canvas,
    div, prelude::*, px, rgb, size,
};
use gpui_3d::{
    AffineTransform, Camera, EvaluatedScene, Interpolation, Keyframe, Material, Mesh, Node,
    NodeHandle, OrbitController, Projection, RotationTrack, SceneGraph, SubtreeInstance,
    TransformTrack, VectorTrack, viewport3d,
};
use gpui_platform::application;
use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant},
};

const ANIMATION_LENGTH: Duration = Duration::from_secs(4);

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

struct SceneDemo {
    graph: SceneGraph,
    evaluated: EvaluatedScene,
    instances: Vec<SubtreeInstance>,
    body: NodeHandle,
    selected: usize,
    hovered: Option<usize>,
    raised: [bool; 3],
    tinted: [bool; 3],
    hidden: [bool; 3],
    controls: OrbitController,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    tracks: [TransformTrack; 3],
    position: Duration,
    playing: bool,
    last_frame: Instant,
    _activation: gpui::Subscription,
}
impl SceneDemo {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let geometry = Mesh::cube();
        let mut source = SceneGraph::new();
        let root = source.insert(None, Node::new().id("assembly")).unwrap();
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
                    .mesh(geometry, Material::color(rgb(0x526a87)))
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
        Self {
            graph,
            evaluated,
            instances,
            body,
            selected: 1,
            hovered: None,
            raised: [false; 3],
            tinted: [false; 3],
            hidden: [false; 3],
            controls: OrbitController::new(camera).unwrap(),
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
    fn evaluate_pose(&mut self) {
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
                if this
                    .controls
                    .scroll(f32::from(event.delta.pixel_delta(px(20.)).y))
                    .unwrap_or(false)
                {
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .child(
                viewport3d("scene", self.evaluated.scene(self.controls.camera()))
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
                    })),
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
        div().size_full().p_6().flex().flex_col().gap_4().bg(rgb(0x0b1422)).text_color(rgb(0xeaf2fc))
            .child(div().text_size(px(30.)).child("Shared shapes, independent nodes"))
            .child(div().text_color(rgb(0x9eb1cb)).child("Click an assembly to select · Right-drag to orbit · Middle-drag to pan · Scroll to zoom"))
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
                    let mut camera = this.controls.camera();
                    let distance = camera.eye.iter().zip(camera.target).map(|(a,b)| (a-b).powi(2)).sum::<f32>().sqrt();
                    camera.projection = match camera.projection {
                        Projection::Perspective { vertical_fov } => Projection::Orthographic { vertical_size: 2. * distance * (vertical_fov * 0.5).tan() },
                        Projection::Orthographic { vertical_size } => Projection::Perspective { vertical_fov: 2. * (vertical_size / (2. * distance)).atan() },
                    };
                    this.controls.set_camera(camera).unwrap(); cx.notify();
                })))
                .child(self.button("frame", "Frame selected", false).on_click(cx.listener(|this, _, _, cx| {
                    let rect = this.bounds.get();
                    let aspect = if rect.size.height > px(0.) { rect.size.width / rect.size.height } else { 1.5 };
                    if let Some(bounds) = this.evaluated.node(this.instances[this.selected].root()).and_then(|node| node.subtree_bounds) {
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
            .child(stage)
            .child(div().text_sm().text_color(rgb(0xa4bad2)).child(format!("Instance {} selected · 3 editable subtrees · 1 shared mesh allocation", self.selected + 1)))
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
