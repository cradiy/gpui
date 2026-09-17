use gpui::{
    App, Bounds, Context, MouseButton, Pixels, Point, Render, Window, WindowBounds, WindowOptions,
    div, prelude::*, px, rgb, size,
};
use gpui_3d::{
    AffineTransform, Camera, EditHiddenStyle, EditLine, EditOcclusionGroup, EditPoint, EditStyle,
    Material, Mesh, Object, Projection, Scene, Scene3dChannels, Scene3dReadbackConfig,
    Scene3dReadbackRegion, SphereOptions, ViewportPickCapture, viewport3d,
};
use gpui_platform::application;
use std::collections::{BTreeMap, BTreeSet};

struct Editing {
    cage: Mesh,
    surface: Mesh,
    blocker: Mesh,
    points: Vec<[f32; 3]>,
    edges: Vec<[usize; 2]>,
    capture: ViewportPickCapture,
    pending: Option<gpui_wgpu::Scene3dReadback>,
    selected: u32,
    status: String,
    yaw: f32,
    pitch: f32,
    drag: Option<Point<Pixels>>,
    orthographic: bool,
    hidden: EditHiddenStyle,
    obstruction: bool,
}

impl Editing {
    fn new() -> Self {
        let cage = Mesh::sphere(SphereOptions {
            radius: 1.,
            segments: [12, 6],
        })
        .unwrap();
        let surface = Mesh::sphere(SphereOptions {
            radius: 1.,
            segments: [64, 32],
        })
        .unwrap();
        let mut unique = BTreeMap::new();
        let mut points = Vec::new();
        let vertices: Vec<_> = cage
            .vertices()
            .iter()
            .map(|v| {
                let key = v.position.map(|v| (v * 100000.).round() as i32);
                *unique.entry(key).or_insert_with(|| {
                    let index = points.len();
                    points.push(v.position);
                    index
                })
            })
            .collect();
        let mut edges = BTreeSet::new();
        for triangle in cage.indices().chunks_exact(3) {
            for (a, b) in [
                (triangle[0], triangle[1]),
                (triangle[1], triangle[2]),
                (triangle[2], triangle[0]),
            ] {
                let (a, b) = (vertices[a as usize], vertices[b as usize]);
                if a != b {
                    edges.insert([a.min(b), a.max(b)]);
                }
            }
        }
        Self {
            cage,
            surface,
            blocker: Mesh::cube(),
            points,
            edges: edges.into_iter().collect(),
            capture: ViewportPickCapture::new(256 * 1024 * 1024),
            pending: None,
            selected: 0,
            status: "Click a point or edge".into(),
            yaw: 0.4,
            pitch: 0.25,
            drag: None,
            orthographic: false,
            hidden: EditHiddenStyle::Dashed { dash: 5., gap: 5. },
            obstruction: true,
        }
    }

    fn scene(&self) -> Scene {
        let mut camera = Camera::orbit(self.yaw, self.pitch, 4.5);
        if self.orthographic {
            camera.projection = Projection::Orthographic { vertical_size: 3.2 };
        }
        let style = |id, size| EditStyle {
            hidden: self.hidden,
            hidden_color: rgb(0x233e51),
            depth_tolerance: 0.0001,
            ..EditStyle::new(
                size,
                if self.selected == id {
                    rgb(0xffc267)
                } else {
                    rgb(0x70d4e9)
                },
            )
        };
        let group = EditOcclusionGroup::new(1, ["surface".into()])
            .mesh(self.cage.clone(), AffineTransform::IDENTITY)
            .lines(self.edges.iter().enumerate().map(|(i, [a, b])| {
                let id = i as u32 + 1;
                EditLine {
                    id,
                    start: self.points[*a],
                    end: self.points[*b],
                    style: style(id, 1.8),
                }
            }))
            .points(self.points.iter().enumerate().map(|(i, p)| {
                let id = self.edges.len() as u32 + i as u32 + 1;
                EditPoint {
                    id,
                    position: *p,
                    style: style(id, 5.),
                }
            }));
        let mut scene = Scene::new().camera(camera).object(
            Object::new(self.surface.clone(), Material::color(rgb(0x264250))).id("surface"),
        );
        if self.obstruction {
            scene = scene.object(
                Object::new(self.blocker.clone(), Material::color(rgb(0xb57c50)))
                    .id("blocker")
                    .position([0.7, -0.2, 0.9])
                    .scale([0.28, 1.5, 0.25]),
            );
        }
        scene.edit_occlusion([group])
    }

    fn click(&mut self, position: Point<Pixels>) {
        self.pending = None;
        let request = (|| -> anyhow::Result<_> {
            let Some(frame) = self.capture.frame()? else {
                return Ok(None);
            };
            let Some(pixel) = frame.pixel_at(position) else {
                return Ok(None);
            };
            let Some(elements) = frame.occlusion_groups().first().and_then(|g| g.elements()) else {
                return Ok(None);
            };
            Ok(Some(elements.readback_region(
                Scene3dReadbackRegion {
                    origin: pixel,
                    size: [1; 2],
                },
                Scene3dReadbackConfig {
                    channels: Scene3dChannels::OBJECT_ID,
                    max_staging_bytes: Some(256),
                    max_cpu_bytes: Some(4),
                },
            )?))
        })();
        match request {
            Ok(pending) => self.pending = pending,
            Err(error) => self.status = error.to_string(),
        }
    }
}

impl Render for Editing {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(pending) = &mut self.pending {
            match pending.try_read() {
                Ok(Some(pixels)) => {
                    self.selected = pixels.object_ids.unwrap()[0];
                    self.pending = None;
                    self.status = if self.selected == 0 {
                        "No element".into()
                    } else {
                        format!("Element {} selected", self.selected)
                    };
                }
                Ok(None) => window.request_animation_frame(),
                Err(error) => {
                    self.status = error.to_string();
                    self.pending = None;
                }
            }
        }
        let button = |id, label| {
            div()
                .id(id)
                .px_4()
                .py_2()
                .rounded_md()
                .bg(rgb(0x243c50))
                .cursor_pointer()
                .child(label)
        };
        let hidden_label = match self.hidden {
            EditHiddenStyle::Hide => "Hidden",
            EditHiddenStyle::Solid => "Solid",
            EditHiddenStyle::Dashed { .. } => "Dashed",
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .p_5()
            .gap_4()
            .bg(rgb(0x0d1620))
            .text_color(rgb(0xe2edf6))
            .child(div().text_xl().child("Edit overlays"))
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(button("hidden", "Hidden style").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.hidden = match this.hidden {
                                EditHiddenStyle::Hide => {
                                    EditHiddenStyle::Dashed { dash: 5., gap: 5. }
                                }
                                EditHiddenStyle::Dashed { .. } => EditHiddenStyle::Solid,
                                EditHiddenStyle::Solid => EditHiddenStyle::Hide,
                            };
                            this.pending = None;
                            cx.notify();
                        },
                    )))
                    .child(
                        button(
                            "projection",
                            if self.orthographic {
                                "Orthographic"
                            } else {
                                "Perspective"
                            },
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.orthographic = !this.orthographic;
                            this.pending = None;
                            cx.notify();
                        })),
                    )
                    .child(button("blocker", "Toggle blocker").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.obstruction = !this.obstruction;
                            this.pending = None;
                            cx.notify();
                        },
                    ))),
            )
            .child(
                div()
                    .id("editing-camera")
                    .debug_selector(|| "editing-camera".into())
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .bg(rgb(0x162330))
                    .overflow_hidden()
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, event: &gpui::MouseDownEvent, _, _| {
                            this.drag = Some(event.position);
                            this.pending = None;
                        }),
                    )
                    .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                        if let Some(previous) = this.drag {
                            if event.pressed_button == Some(MouseButton::Right) {
                                let delta = event.position - previous;
                                this.yaw -= f32::from(delta.x) * 0.008;
                                this.pitch =
                                    (this.pitch + f32::from(delta.y) * 0.008).clamp(-1.4, 1.4);
                                this.drag = Some(event.position);
                                cx.notify();
                            } else {
                                this.drag = None;
                            }
                        }
                    }))
                    .on_mouse_up(
                        MouseButton::Right,
                        cx.listener(|this, _, _, _| this.drag = None),
                    )
                    .on_mouse_up_out(
                        MouseButton::Right,
                        cx.listener(|this, _, _, _| this.drag = None),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                            this.click(event.position);
                            cx.notify();
                        }),
                    )
                    .child(
                        viewport3d("editing", self.scene())
                            .size_full()
                            .pick_capture(self.capture.clone()),
                    ),
            )
            .child(format!(
                "Right-drag to orbit · {hidden_label} · {}",
                self.status
            ))
    }
}

fn main() {
    application().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1100.), px(820.)),
                    cx,
                ))),
                ..Default::default()
            },
            |_, cx| cx.new(|_| Editing::new()),
        )
        .expect("failed to open edit overlay example");
    });
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use super::*;
    use gpui::{Modifiers, TestAppContext};

    #[gpui::test]
    fn viewport_mouse_input(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, _| Editing::new());
        let position = cx.debug_bounds("editing-camera").unwrap().center();

        cx.simulate_mouse_down(position, MouseButton::Right, Modifiers::none());
        assert_eq!(cx.read_entity(&view, |view, _| view.drag), Some(position));
        cx.simulate_mouse_up(position, MouseButton::Right, Modifiers::none());
        assert_eq!(cx.read_entity(&view, |view, _| view.drag), None);

        cx.simulate_click(position, Modifiers::none());
        cx.run_until_parked();
    }
}
