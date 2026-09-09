use gpui::{
    App, Bounds, Context, MouseButton, Pixels, Point, Render, Window, WindowBounds, WindowOptions,
    div, prelude::*, px, rgb, size,
};
use gpui_3d::{
    AffineTransform, Camera, EvaluatedScene, Material, Mesh, Node, NodeHandle, ReparentMode,
    SceneGraph, viewport3d,
};
use gpui_platform::application;

fn local(position: [f32; 3], angle: f32, scale: [f32; 3]) -> AffineTransform {
    AffineTransform::from_trs(
        position,
        [0., (angle * 0.5).sin(), 0., (angle * 0.5).cos()],
        scale,
    )
    .expect("finite example transform")
}

struct Hierarchy {
    graph: SceneGraph,
    evaluated: EvaluatedScene,
    groups: [NodeHandle; 2],
    angles: [f32; 2],
    coral: Option<NodeHandle>,
    parent: usize,
    keep_world: bool,
    hidden: bool,
    names: Vec<(NodeHandle, &'static str)>,
    hovered: Option<NodeHandle>,
    selected: Option<NodeHandle>,
    yaw: f32,
    pitch: f32,
    distance: f32,
    drag: Option<Point<Pixels>>,
    _activation: gpui::Subscription,
}

impl Hierarchy {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut graph = SceneGraph::new();
        let groups = [-1.7, 1.7].map(|x| {
            graph
                .insert(None, Node::new().transform(local([x, 0., 0.], 0., [1.; 3])))
                .expect("root node")
        });
        let mut names = vec![(groups[0], "Group A"), (groups[1], "Group B")];
        let geometry = Mesh::cube();
        for (i, (color, base_name, cube_name, child_name)) in [
            (0x8bd6e6, "A / base", "A / cube", "A / child"),
            (0xc6b2f5, "B / base", "B / cube", "B / child"),
        ]
        .into_iter()
        .enumerate()
        {
            let base = graph
                .insert(
                    Some(groups[i]),
                    Node::new()
                        .id(base_name)
                        .mesh(geometry.clone(), Material::color(rgb(0x344b66)))
                        .transform(local([0., -0.9, 0.], 0., [2.3, 0.12, 2.3])),
                )
                .unwrap();
            let cube = graph
                .insert(
                    Some(groups[i]),
                    Node::new()
                        .id(cube_name)
                        .mesh(geometry.clone(), Material::color(rgb(color)))
                        .transform(local([0., -0.2, 0.], 0.15, [1.; 3])),
                )
                .unwrap();
            let child = graph
                .insert(
                    Some(cube),
                    Node::new()
                        .id(child_name)
                        .mesh(geometry.clone(), Material::color(rgb(color)))
                        .transform(local([-0.65, 0.9, 0.], 0.4, [0.45; 3])),
                )
                .unwrap();
            names.extend([(base, base_name), (cube, cube_name), (child, child_name)]);
        }
        let coral = graph.insert(Some(groups[0]), Self::coral_node()).unwrap();
        names.push((coral, "Coral"));
        let evaluated = graph.evaluate().unwrap();
        Self {
            graph,
            evaluated,
            groups,
            angles: [0.; 2],
            coral: Some(coral),
            parent: 0,
            keep_world: true,
            hidden: false,
            names,
            hovered: None,
            selected: None,
            yaw: 0.25,
            pitch: 0.25,
            distance: 9.,
            drag: None,
            _activation: cx.observe_window_activation(window, |this, window, _| {
                if !window.is_window_active() {
                    this.drag = None;
                }
            }),
        }
    }

    fn coral_node() -> Node {
        Node::new()
            .id("coral")
            .mesh(Mesh::cube(), Material::color(rgb(0xf19983)))
            .transform(local([0.7, 1.25, 0.2], 0.25, [0.55; 3]))
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.evaluated = self.graph.evaluate().expect("valid hierarchy");
        self.hovered = None;
        cx.notify();
    }

    fn label(&self, handle: Option<NodeHandle>) -> &'static str {
        self.names
            .iter()
            .find(|(id, _)| Some(*id) == handle)
            .map_or("None", |(_, name)| *name)
    }

    fn button(
        &self,
        id: &'static str,
        label: impl Into<gpui::SharedString>,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .px_4()
            .py_2()
            .rounded(px(10.))
            .bg(rgb(0x243952))
            .hover(|style| style.bg(rgb(0x35506c)))
            .cursor_pointer()
            .child(label.into())
    }
}

impl Render for Hierarchy {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let scene = self
            .evaluated
            .scene(Camera::orbit(self.yaw, self.pitch, self.distance));
        let coral_position = self
            .coral
            .and_then(|node| self.evaluated.node(node))
            .map(|node| node.world.transform_point([0.; 3]));
        div().size_full().p_6().flex().flex_col().gap_4().bg(rgb(0x0c1523)).text_color(rgb(0xeaf2fc))
            .child(div().flex().flex_col().gap_2()
                .child(div().text_size(px(30.)).child("Connected in space"))
                .child(div().text_color(rgb(0x9baec7)).child("Rotate either group · Move coral between parents · Right-drag to orbit")))
            .child(div().id("camera").relative().flex_1().min_h_0().w_full().rounded(px(24.))
                .overflow_hidden().bg(rgb(0x142238))
                .on_mouse_down(MouseButton::Right, cx.listener(|this, event: &gpui::MouseDownEvent, _, _| {
                    this.drag = Some(event.position);
                }))
                .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                    if let Some(previous) = this.drag {
                        if event.pressed_button == Some(MouseButton::Right) {
                            let delta = event.position - previous;
                            this.yaw -= f32::from(delta.x) * 0.008;
                            this.pitch = (this.pitch + f32::from(delta.y) * 0.008).clamp(-1.3, 1.3);
                            this.drag = Some(event.position);
                            cx.notify();
                        } else { this.drag = None; }
                    }
                }))
                .on_mouse_up(MouseButton::Right, cx.listener(|this, _, _, _| this.drag = None))
                .on_mouse_up_out(MouseButton::Right, cx.listener(|this, _, _, _| this.drag = None))
                .on_hover(cx.listener(|this, hovered: &bool, _, _| { if !hovered { this.drag = None; } }))
                .on_scroll_wheel(cx.listener(|this, event: &gpui::ScrollWheelEvent, _, cx| {
                    this.distance = (this.distance * (f32::from(event.delta.pixel_delta(px(20.)).y) * 0.002).exp()).clamp(4., 18.);
                    cx.notify();
                }))
                .child(viewport3d("hierarchy", scene).size_full()
                    .on_object_hover(cx.listener(|this, hit: &Option<gpui_3d::Hit>, _, cx| {
                        let hovered = hit.as_ref().and_then(|hit| hit.node);
                        if this.hovered != hovered { this.hovered = hovered; cx.notify(); }
                    }))
                    .on_object_click(cx.listener(|this, hit: &gpui_3d::Hit, _, cx| {
                        this.selected = hit.node;
                        cx.notify();
                    }))))
            .child(div().flex().flex_wrap().items_center().gap_3()
                .children([0, 1].map(|i| {
                    self.button(if i == 0 { "rotate-a" } else { "rotate-b" }, if i == 0 { "Rotate A" } else { "Rotate B" })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.angles[i] += std::f32::consts::FRAC_PI_6;
                            this.graph.set_transform(this.groups[i], local([if i == 0 { -1.7 } else { 1.7 }, 0., 0.], this.angles[i], [1.; 3])).unwrap();
                            this.refresh(cx);
                        }))
                }))
                .child(self.button("visibility", if self.hidden { "Show A" } else { "Hide A" })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.hidden = !this.hidden;
                        this.graph.set_visible(this.groups[0], !this.hidden).unwrap();
                        this.refresh(cx);
                    })))
                .child(self.button("policy", if self.keep_world { "Keep world" } else { "Keep local" })
                    .on_click(cx.listener(|this, _, _, cx| { this.keep_world = !this.keep_world; cx.notify(); })))
                .child(self.button("reparent", if self.parent == 0 { "Move coral → B" } else { "Move coral → A" })
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(coral) = this.coral {
                            this.parent = 1 - this.parent;
                            this.graph.reparent(coral, Some(this.groups[this.parent]), if this.keep_world { ReparentMode::KeepWorld } else { ReparentMode::KeepLocal }).unwrap();
                            this.refresh(cx);
                        }
                    })))
                .child(self.button("remove", if self.coral.is_some() { "Remove coral" } else { "Restore coral" })
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(coral) = this.coral.take() {
                            this.graph.remove_subtree(coral).unwrap();
                            this.names.retain(|(node, _)| *node != coral);
                            if this.selected == Some(coral) { this.selected = None; }
                        } else {
                            let coral = this.graph.insert(Some(this.groups[this.parent]), Self::coral_node()).unwrap();
                            this.coral = Some(coral);
                            this.names.push((coral, "Coral"));
                        }
                        this.refresh(cx);
                    })))
                .child(self.button("reset", "Reset").on_click(cx.listener(|this, _, window, cx| {
                    *this = Self::new(window, cx);
                    cx.notify();
                }))))
            .child(div().flex().flex_wrap().gap_4().text_sm().text_color(rgb(0xa6bbd2))
                .child(format!("Pointer: {}", self.label(self.hovered)))
                .child(format!("Selected: {}", self.label(self.selected)))
                .child(format!("Coral parent: {}", if self.parent == 0 { "A" } else { "B" }))
                .child(coral_position.map_or_else(|| "Coral removed".into(), |[x,y,z]| format!("World: {x:.2}, {y:.2}, {z:.2}"))))
            .child(div().text_sm().text_color(rgb(0x8298b3)).child(if self.keep_world {
                "Keep world preserves position when changing parent. Rotate the new parent to see the attachment."
            } else {
                "Keep local preserves the offset within the parent. Changing parent moves coral to that group's space."
            }))
            .when(!window.supports_scene3d(), |root| root.child("3D viewports are unavailable on this renderer."))
    }
}

fn main() {
    application().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1220.), px(900.)),
                    cx,
                ))),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| Hierarchy::new(window, cx)),
        )
        .expect("failed to open hierarchy example");
    });
}
