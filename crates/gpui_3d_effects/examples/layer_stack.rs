use std::time::Instant;

use gpui::{
    App, Bounds, Context, MouseButton, Pixels, Point, Render, Subscription, Window, WindowBounds,
    WindowOptions, div, prelude::*, px, rgb, size,
};
use gpui_3d::{Camera, Material, Mesh, Object, Scene, viewport3d};
use gpui_3d_effects::LayerStack;
use gpui_platform::application;

struct Demo {
    stack: LayerStack,
    plane: Mesh,
    progress: f32,
    target: f32,
    last_frame: Instant,
    yaw: f32,
    pitch: f32,
    drag: Option<Point<Pixels>>,
    _activation: Subscription,
}

impl Demo {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            stack: LayerStack::new(5),
            plane: Mesh::plane(),
            progress: 0.,
            target: 1.,
            last_frame: Instant::now(),
            yaw: 0.45,
            pitch: 0.18,
            drag: None,
            _activation: cx.observe_window_activation(window, |this, _, cx| {
                this.last_frame = Instant::now();
                this.drag = None;
                cx.notify();
            }),
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
                    "expand" => this.target = 1.,
                    "collapse" => this.target = 0.,
                    "back" | "forward" => {
                        this.progress =
                            (this.progress + if id == "back" { -0.1 } else { 0.1 }).clamp(0., 1.);
                        this.target = this.progress;
                    }
                    "camera" => {
                        this.yaw = 0.45;
                        this.pitch = 0.18;
                    }
                    _ => {}
                }
                this.last_frame = Instant::now();
                cx.notify();
            }))
    }

    fn surface(&self) -> impl IntoElement {
        div()
            .size_full()
            .rounded(px(22.))
            .bg(rgb(0xffffff))
            .text_color(rgb(0x203344))
            .p_6()
            .flex()
            .flex_col()
            .gap_5()
            .child(div().text_sm().child("SURFACE"))
            .child(
                div()
                    .w(px(64.))
                    .h(px(64.))
                    .rounded(px(16.))
                    .bg(rgb(0x4c6478)),
            )
            .child(div().text_size(px(32.)).child("Layer"))
            .child(div().text_base().child("Ordinary UI in space"))
            .child(div().h(px(8.)).w_full().rounded_full().bg(rgb(0xd0d8df)))
            .child(div().h(px(8.)).w(px(160.)).rounded_full().bg(rgb(0xd0d8df)))
            .child(div().flex_1())
            .child(div().text_sm().child("Position · Depth · Rotation"))
    }
}

impl Render for Demo {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
        if window.is_window_active() && window.supports_scene3d() && self.progress != self.target {
            let step = now.duration_since(self.last_frame).as_secs_f32().min(0.05) / 0.9;
            if self.progress < self.target {
                self.progress = (self.progress + step).min(self.target);
            } else {
                self.progress = (self.progress - step).max(self.target);
            }
            if self.progress != self.target {
                window.request_animation_frame();
            }
        }
        self.last_frame = now;
        let eased = self.progress * self.progress * (3. - 2. * self.progress);
        let colors = [0xf0dfcb, 0xc6e5dd, 0xd0def2, 0xded5ec, 0xe7d1d6];
        let mut scene = Scene::new().camera(Camera::orbit(self.yaw, self.pitch, 6.2));
        for (pose, color) in self.stack.sample(eased).zip(colors) {
            scene = scene.object(
                Object::new(self.plane.clone(), Material::ui().tint(rgb(color)))
                    .transform(pose)
                    .scale([1.5, 2.0625, 1.]),
            );
        }
        div()
            .size_full()
            .p_6()
            .flex()
            .flex_col()
            .gap_4()
            .bg(rgb(0x101824))
            .text_color(rgb(0xe3edf5))
            .child(div().text_size(px(28.)).child("Layer stack"))
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0x94a9bb))
                    .child("Expand and collapse · Right-drag to rotate the view"),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_3()
                    .items_center()
                    .child(self.button("expand", "Expand", cx))
                    .child(self.button("collapse", "Collapse", cx))
                    .child(self.button("back", "−10%", cx))
                    .child(self.button("forward", "+10%", cx))
                    .child(self.button("camera", "Reset view", cx))
                    .child(format!("{:.0}%", self.progress * 100.)),
            )
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
                        viewport3d("layers", scene)
                            .size_full()
                            .color_samples(4)
                            .ui_texture_size(size(px(320.), px(440.)))
                            .ui_texture_scale(2.)
                            .ui_texture(self.surface()),
                    ),
            )
            .when(!window.supports_scene3d(), |root| {
                root.child("3D rendering is unavailable on this window.")
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
        .expect("layer stack window");
    });
}
