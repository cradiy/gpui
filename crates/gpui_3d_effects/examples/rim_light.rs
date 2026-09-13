use gpui::{
    App, Bounds, Context, MouseButton, Pixels, Point, Render, Subscription, Window, WindowBounds,
    WindowOptions, div, prelude::*, px, rgb, size,
};
use gpui_3d::{
    Camera, Light, Material, Mesh, MeshPass, Object, Scene, SphereOptions, WgpuContext, viewport3d,
};
use gpui_3d_effects::RimLight;
use gpui_platform::application;

struct Demo {
    rim: Option<RimLight>,
    pass: Option<MeshPass>,
    error: Option<String>,
    mesh: Mesh,
    enabled: bool,
    warm: bool,
    intensity: f32,
    falloff: f32,
    yaw: f32,
    pitch: f32,
    drag: Option<Point<Pixels>>,
    _activation: Subscription,
}

impl Demo {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let result = WgpuContext::for_window(window)
            .ok_or_else(|| anyhow::anyhow!("A WGPU window is required"))
            .and_then(RimLight::new);
        let (rim, error) = match result {
            Ok(rim) => (Some(rim), None),
            Err(error) => (None, Some(error.to_string())),
        };
        let mut demo = Self {
            rim,
            pass: None,
            error,
            mesh: Mesh::sphere(SphereOptions {
                radius: 1.,
                segments: [128, 64],
            })
            .expect("sphere geometry"),
            enabled: true,
            warm: false,
            intensity: 1.2,
            falloff: 3.,
            yaw: 0.3,
            pitch: 0.18,
            drag: None,
            _activation: cx.observe_window_activation(window, |this, _, _| this.drag = None),
        };
        demo.update_pass();
        demo
    }

    fn update_pass(&mut self) {
        if let Some(rim) = &self.rim {
            match rim
                .clone()
                .color(rgb(if self.warm { 0xffd7ac } else { 0xc6e6ff }))
                .intensity(self.intensity)
                .falloff(self.falloff)
                .pass()
            {
                Ok(pass) => {
                    self.pass = Some(pass);
                    self.error = None;
                }
                Err(error) => {
                    self.pass = None;
                    self.error = Some(error.to_string());
                }
            }
        }
    }

    fn button(
        &self,
        id: &'static str,
        label: &'static str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id(id)
            .px_4()
            .py_2()
            .rounded_lg()
            .bg(rgb(0x263b50))
            .hover(|style| style.bg(rgb(0x36536c)))
            .cursor_pointer()
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                match id {
                    "toggle" => this.enabled = !this.enabled,
                    "color" => this.warm = !this.warm,
                    "softer" => this.falloff = (this.falloff - 0.5).max(0.5),
                    "tighter" => this.falloff = (this.falloff + 0.5).min(12.),
                    "dimmer" => this.intensity = (this.intensity - 0.2).max(0.),
                    "brighter" => this.intensity = (this.intensity + 0.2).min(4.),
                    "view" => {
                        this.yaw = 0.3;
                        this.pitch = 0.18;
                    }
                    _ => {}
                }
                if !matches!(id, "toggle" | "view") {
                    this.update_pass();
                }
                cx.notify();
            }))
    }
}

impl Render for Demo {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let material =
            Material::color(rgb(0x527d94)).mesh_passes(self.pass.clone().filter(|_| self.enabled));
        let scene = Scene::new()
            .camera(Camera::orbit(self.yaw, self.pitch, 4.8))
            .light(Light {
                direction: [-0.7, 1., 1.3],
                color: rgb(0xf3f8ff),
                intensity: 1.8,
                ambient: 0.3,
            })
            .object(
                Object::new(self.mesh.clone(), material)
                    .scale([1.15, 0.78, 0.65])
                    .rotation([0.15, 0.25, -0.35]),
            );
        div()
            .size_full()
            .p_6()
            .flex()
            .flex_col()
            .gap_4()
            .bg(rgb(0x101824))
            .text_color(rgb(0xe3edf5))
            .child(div().text_size(px(28.)).child("Rim light"))
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0x94a9bb))
                    .child("View-dependent edge light · Right-drag to rotate the view"),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_3()
                    .items_center()
                    .child(self.button(
                        "toggle",
                        if self.enabled { "Rim: On" } else { "Rim: Off" },
                        cx,
                    ))
                    .child(self.button("color", if self.warm { "Warm" } else { "Cool" }, cx))
                    .child(self.button("softer", "Broader", cx))
                    .child(self.button("tighter", "Tighter", cx))
                    .child(self.button("dimmer", "Dimmer", cx))
                    .child(self.button("brighter", "Brighter", cx))
                    .child(self.button("view", "Reset view", cx)),
            )
            .child(div().text_sm().text_color(rgb(0x94a9bb)).child(format!(
                "Intensity {:.1} · Falloff {:.1}",
                self.intensity, self.falloff
            )))
            .child(
                div()
                    .id("stage")
                    .relative()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .rounded_xl()
                    .overflow_hidden()
                    .bg(rgb(0x172535))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, event: &gpui::MouseDownEvent, _, _| {
                            this.drag = Some(event.position);
                        }),
                    )
                    .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                        if let Some(previous) = this.drag {
                            if event.pressed_button == Some(MouseButton::Right) {
                                let delta = event.position - previous;
                                this.yaw -= f32::from(delta.x) * 0.008;
                                this.pitch =
                                    (this.pitch + f32::from(delta.y) * 0.008).clamp(-1.2, 1.2);
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
                    .on_hover(cx.listener(|this, hovered: &bool, _, _| {
                        if !hovered {
                            this.drag = None;
                        }
                    }))
                    .child(
                        viewport3d("rim", scene)
                            .size_full()
                            .color_samples(4)
                            .resolution_scale(1.5),
                    ),
            )
            .when_some(self.error.clone(), |root, error| {
                root.child(div().text_color(rgb(0xffaa88)).child(error))
            })
    }
}

fn main() {
    application().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1040.), px(760.)),
                    cx,
                ))),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| Demo::new(window, cx)),
        )
        .expect("rim light window");
    });
}
