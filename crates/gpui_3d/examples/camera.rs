use gpui::{
    App, Bounds, Context, MouseButton, Pixels, Render, Window, WindowBounds, WindowOptions, canvas,
    div, prelude::*, px, rgb, size,
};
use gpui_3d::{
    AffineTransform, Camera, EvaluatedScene, Material, Mesh, Node, NodeHandle, OrbitController,
    OrbitSettings, Projection, SceneGraph, viewport3d,
};
use gpui_platform::application;
use std::{cell::Cell, rc::Rc};

struct CameraDemo {
    graph: SceneGraph,
    evaluated: EvaluatedScene,
    items: Vec<(NodeHandle, &'static str, u32)>,
    selected: Option<NodeHandle>,
    hovered: Option<NodeHandle>,
    controls: OrbitController,
    viewport: Rc<Cell<Bounds<Pixels>>>,
    _activation: gpui::Subscription,
}

impl CameraDemo {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut graph = SceneGraph::new();
        let geometry = Mesh::cube();
        let items = [
            ("Near / coral", [-1.5, 0., 2.], 0xf09e8e),
            ("Near / gold", [1.5, 0., 2.], 0xf4cf89),
            ("Middle / ice", [-1.5, 0., 0.], 0x8dd8e8),
            ("Middle / blue", [1.5, 0., 0.], 0x96bcef),
            ("Far / lilac", [-1.5, 0., -2.], 0xcbb1ef),
            ("Far / mint", [1.5, 0., -2.], 0x9ce0c4),
        ]
        .into_iter()
        .map(|(name, position, color)| {
            let handle = graph
                .insert(
                    None,
                    Node::new()
                        .id(name)
                        .mesh(geometry.clone(), Material::color(rgb(color)))
                        .transform(AffineTransform::from_translation(position).unwrap()),
                )
                .unwrap();
            (handle, name, color)
        })
        .collect();
        let evaluated = graph.evaluate().unwrap();
        let mut camera = Camera::orbit(0.35, 0.35, 10.)
            .frame_bounds(evaluated.bounds().unwrap(), 1.5, 1.3)
            .unwrap();
        camera.near = 0.01;
        camera.far = 100.;
        let mut controls = OrbitController::new(camera).unwrap();
        controls
            .set_settings(OrbitSettings {
                distance: 0.5..=30.,
                orthographic_size: 0.2..=40.,
                ..Default::default()
            })
            .unwrap();
        Self {
            graph,
            evaluated,
            items,
            selected: None,
            hovered: None,
            controls,
            viewport: Rc::new(Cell::new(Bounds::default())),
            _activation: cx.observe_window_activation(window, |this, window, _| {
                if !window.is_window_active() {
                    this.controls.cancel_drag();
                }
            }),
        }
    }

    fn aspect(&self) -> f32 {
        let bounds = self.viewport.get();
        if bounds.size.width > px(0.) && bounds.size.height > px(0.) {
            bounds.size.width / bounds.size.height
        } else {
            1.5
        }
    }
    fn distance(&self) -> f32 {
        self.controls
            .camera()
            .eye
            .iter()
            .zip(self.controls.camera().target)
            .map(|(a, b)| (a - b).powi(2))
            .sum::<f32>()
            .sqrt()
    }
    fn orient(&mut self, yaw: f32, pitch: f32) {
        let distance = self.distance();
        let direction = [
            yaw.sin() * pitch.cos(),
            pitch.sin(),
            yaw.cos() * pitch.cos(),
        ];
        let mut camera = self.controls.camera();
        camera.eye = std::array::from_fn(|i| camera.target[i] + direction[i] * distance);
        self.controls.set_camera(camera).unwrap();
    }
    fn frame(&mut self, selected: bool) {
        let bounds = if selected {
            self.selected
                .and_then(|node| self.evaluated.node(node))
                .and_then(|node| node.bounds)
        } else {
            self.evaluated.bounds()
        };
        if let Some(bounds) = bounds {
            let mut camera = self
                .controls
                .camera()
                .frame_bounds(bounds, self.aspect(), 1.3)
                .unwrap();
            camera.near = 0.01;
            camera.far = 100.;
            self.controls.set_camera(camera).unwrap();
        }
    }
    fn label(&self, node: Option<NodeHandle>) -> &'static str {
        self.items
            .iter()
            .find(|(id, _, _)| Some(*id) == node)
            .map_or("None", |(_, name, _)| *name)
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

impl Render for CameraDemo {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let camera = self.controls.camera();
        let orthographic = matches!(camera.projection, Projection::Orthographic { .. });
        let viewport_bounds = self.viewport.clone();
        let view_id = cx.entity_id();
        let scene = self.evaluated.scene(camera);
        let projection_info = match camera.projection {
            Projection::Perspective { vertical_fov } => {
                format!("Vertical FOV: {:.0}°", vertical_fov.to_degrees())
            }
            Projection::Orthographic { vertical_size } => {
                format!("Vertical span: {vertical_size:.2} units")
            }
        };
        let selection_info = self
            .selected
            .and_then(|node| self.evaluated.node(node))
            .and_then(|node| {
                camera
                    .world_to_screen(self.viewport.get(), node.world.transform_point([0.; 3]))
                    .ok()
                    .flatten()
            })
            .map_or_else(
                || "Select a cube to inspect its projection".to_string(),
                |p| {
                    format!(
                        "Screen: {:.0}, {:.0} · Forward depth: {:.2} · {}",
                        f32::from(p.position.x),
                        f32::from(p.position.y),
                        p.depth,
                        if p.in_frustum {
                            "Inside frustum"
                        } else {
                            "Outside frustum"
                        }
                    )
                },
            );
        div().size_full().p_6().flex().flex_col().gap_4().bg(rgb(0x0b1422)).text_color(rgb(0xeaf2fc))
            .child(div().flex().flex_col().gap_2()
                .child(div().text_size(px(30.)).child("A matter of perspective"))
                .child(div().text_color(rgb(0x9eb1cb)).child("Click to select · Right-drag to orbit · Middle-drag to pan · Scroll to dolly / orthographic zoom")))
            .child(div().flex().flex_wrap().gap_3()
                .children([(false, "perspective", "Perspective"), (true, "orthographic", "Orthographic")].map(|(ortho, id, label)| {
                    self.button(id, label, ortho == orthographic).on_click(cx.listener(move |this, _, _, cx| {
                        let mut camera = this.controls.camera();
                        match (camera.projection, ortho) {
                            (Projection::Perspective { vertical_fov }, true) => {
                                camera.projection = Projection::Orthographic {
                                    vertical_size: this.distance() * 2. * (vertical_fov * 0.5).tan(),
                                };
                            }
                            (Projection::Orthographic { vertical_size }, false) => {
                                camera.projection = Projection::default();
                                let distance = vertical_size / (2. * (std::f32::consts::FRAC_PI_4 * 0.5).tan());
                                let backward = camera.axes().unwrap()[2];
                                camera.eye = std::array::from_fn(|i| camera.target[i] + backward[i] * distance);
                            }
                            _ => return,
                        }
                        this.controls.set_camera(camera).unwrap();
                        this.hovered = None;
                        cx.notify();
                    }))
                }))
                .child(self.button("all", "Frame all", false).on_click(cx.listener(|this, _, _, cx| { this.frame(false); cx.notify(); })))
                .child(self.button("selection", "Frame selected", false).on_click(cx.listener(|this, _, _, cx| { this.frame(true); cx.notify(); })))
                .children([(0., 0., "front", "Front"), (0., std::f32::consts::FRAC_PI_2, "top", "Top"), (0.65, 0.4, "oblique", "Oblique")].map(|(yaw, pitch, id, label)| {
                    self.button(id, label, false).on_click(cx.listener(move |this, _, _, cx| {
                        this.orient(yaw, pitch); this.frame(false); cx.notify();
                    }))
                }))
                .children([("lens-in", "Lens +", 0.85), ("lens-out", "Lens −", 1. / 0.85)].map(|(id, label, factor)| {
                    self.button(id, label, false).on_click(cx.listener(move |this, _, _, cx| {
                        this.controls.cancel_drag();
                        if this.controls.zoom(factor).unwrap_or(false) { cx.notify(); }
                    }))
                })))
            .child(div().id("camera").relative().w_full().flex_1().min_h_0().rounded(px(24.)).overflow_hidden().bg(rgb(0x142237))
                .on_mouse_down(MouseButton::Right, cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                    if this.controls.begin_drag(event.button, event.position, this.viewport.get()).unwrap_or(false) { cx.stop_propagation(); }
                }))
                .on_mouse_down(MouseButton::Middle, cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                    if this.controls.begin_drag(event.button, event.position, this.viewport.get()).unwrap_or(false) { cx.stop_propagation(); }
                }))
                .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                    if this.controls.update_drag(event.position, event.pressed_button, this.viewport.get()).unwrap_or(false) { cx.notify(); }
                    if this.controls.is_dragging() { cx.stop_propagation(); }
                }))
                .on_mouse_up(MouseButton::Right, cx.listener(|this, _, _, cx| { if this.controls.end_drag(MouseButton::Right) { cx.stop_propagation(); } }))
                .on_mouse_up_out(MouseButton::Right, cx.listener(|this, _, _, _| { this.controls.end_drag(MouseButton::Right); }))
                .on_mouse_up(MouseButton::Middle, cx.listener(|this, _, _, cx| { if this.controls.end_drag(MouseButton::Middle) { cx.stop_propagation(); } }))
                .on_mouse_up_out(MouseButton::Middle, cx.listener(|this, _, _, _| { this.controls.end_drag(MouseButton::Middle); }))
                .on_hover(cx.listener(|this, hovered: &bool, _, _| { if !hovered { this.controls.cancel_drag(); } }))
                .on_scroll_wheel(cx.listener(|this, event: &gpui::ScrollWheelEvent, _, cx| {
                    if this.controls.scroll(f32::from(event.delta.pixel_delta(px(20.)).y)).unwrap_or(false) { cx.stop_propagation(); cx.notify(); }
                }))
                .child(viewport3d("scene", scene).size_full()
                    .on_object_hover(cx.listener(|this, hit: &Option<gpui_3d::Hit>, _, cx| {
                        let hovered = hit.as_ref().and_then(|hit| hit.node);
                        if hovered != this.hovered { this.hovered = hovered; cx.notify(); }
                    }))
                    .on_object_click(cx.listener(|this, hit: &gpui_3d::Hit, _, cx| {
                        this.selected = hit.node;
                        for &(node, _, color) in &this.items {
                            this.graph.set_material(node, Material::color(rgb(if Some(node) == this.selected { 0xffffff } else { color }))).unwrap();
                        }
                        this.evaluated = this.graph.evaluate().unwrap(); cx.notify();
                    })))
                .child(canvas(move |bounds, _, cx| {
                    if viewport_bounds.replace(bounds) != bounds { cx.notify(view_id); }
                }, |_, _, _, _| {}).absolute().inset_0().size_full()))
            .child(div().flex().flex_wrap().gap_4().text_sm().text_color(rgb(0xa4bad2))
                .child(projection_info).child(format!("Pointer: {}", self.label(self.hovered)))
                .child(format!("Selected: {}", self.label(self.selected))))
            .child(div().text_sm().text_color(rgb(0x849db9)).child(selection_info))
            .when(!window.supports_scene3d(), |root| root.child("3D viewports are unavailable on this renderer."))
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
            |window, cx| cx.new(|cx| CameraDemo::new(window, cx)),
        )
        .expect("failed to open camera example");
    });
}
