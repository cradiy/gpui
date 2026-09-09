use gpui::{
    App, Bounds, Context, MouseButton, Pixels, Render, Window, WindowBounds, WindowOptions, canvas,
    div, prelude::*, px, rgb, size,
};
use gpui_3d::{
    AffineTransform, Camera, EvaluatedScene, Material, Mesh, Node, NodeHandle, OrbitController,
    SceneGraph, SubtreeInstance, viewport3d,
};
use gpui_platform::application;
use std::{cell::Cell, rc::Rc};

fn local(position: [f32; 3], scale: [f32; 3]) -> AffineTransform {
    AffineTransform::from_trs(position, [0., 0., 0., 1.], scale).unwrap()
}

struct Instances {
    graph: SceneGraph,
    evaluated: EvaluatedScene,
    instances: Vec<SubtreeInstance>,
    body: NodeHandle,
    selected: usize,
    raised: [bool; 3],
    tinted: [bool; 3],
    hidden: [bool; 3],
    controls: OrbitController,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    _activation: gpui::Subscription,
}
impl Instances {
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
            raised: [false; 3],
            tinted: [false; 3],
            hidden: [false; 3],
            controls: OrbitController::new(camera).unwrap(),
            bounds: Rc::new(Cell::new(Bounds::default())),
            _activation: cx.observe_window_activation(window, |this, window, _| {
                if !window.is_window_active() {
                    this.controls.cancel_drag();
                }
            }),
        }
    }
    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.evaluated = self.graph.evaluate().unwrap();
        cx.notify();
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
impl Render for Instances {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
                viewport3d("instances", self.evaluated.scene(self.controls.camera()))
                    .size_full()
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
                .children([("one", "Instance 1"), ("two", "Instance 2"), ("three", "Instance 3")].into_iter().enumerate().map(|(index, (id, label))| {
                    self.button(id, label, self.selected == index).on_click(cx.listener(move |this, _, _, cx| { this.selected = index; cx.notify(); }))
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
                .child(self.button("reset", "Reset", false).on_click(cx.listener(|this, _, window, cx| { *this = Self::new(window, cx); cx.notify(); }))))
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
            |window, cx| cx.new(|cx| Instances::new(window, cx)),
        )
        .expect("failed to open instances example");
    });
}
