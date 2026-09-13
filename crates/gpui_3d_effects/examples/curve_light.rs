use std::time::Instant;

use gpui::{
    App, Bounds, Context, MouseButton, Pixels, Point, Render, Subscription, Window, WindowBounds,
    WindowOptions, div, prelude::*, px, rgb, size,
};
use gpui_3d::{Camera, Light, Material, Mesh, Object, Scene, WgpuContext, viewport3d};
use gpui_3d_effects::CurveLight;
use gpui_effects::{BloomOptions, EffectStage, subtree_effect_chain};
use gpui_platform::application;

struct Demo {
    curve: Option<CurveLight>,
    error: Option<String>,
    cube: Mesh,
    progress: f32,
    playing: bool,
    reveal: bool,
    last_frame: Instant,
    yaw: f32,
    pitch: f32,
    drag: Option<Point<Pixels>>,
    _activation: Subscription,
}

impl Demo {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let result = WgpuContext::for_window(window)
            .ok_or_else(|| anyhow::anyhow!("A WGPU window is required"))
            .and_then(|context| {
                CurveLight::new(
                    context,
                    [
                        [-1.8, -0.65, 0.3],
                        [-1.3, 0.5, 0.2],
                        [-0.55, 0.55, -0.7],
                        [0.35, 0., -0.8],
                        [0.25, -0.35, 0.9],
                        [1.25, -0.25, 0.85],
                        [1.8, 0.75, 0.3],
                    ],
                )
            })
            .and_then(|curve| curve.width(0.025));
        let (curve, error) = match result {
            Ok(curve) => (Some(curve), None),
            Err(error) => (None, Some(error.to_string())),
        };
        Self {
            curve,
            error,
            cube: Mesh::cube(),
            progress: 0.,
            playing: true,
            reveal: false,
            last_frame: Instant::now(),
            yaw: 0.15,
            pitch: 0.12,
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
                    "play" => {
                        this.playing = !this.playing;
                        if this.playing && this.progress >= 1. {
                            this.progress = 0.;
                        }
                    }
                    "mode" => {
                        this.reveal = !this.reveal;
                        this.progress = 0.;
                        this.playing = true;
                    }
                    "restart" => {
                        this.progress = 0.;
                        this.playing = true;
                    }
                    "back" | "forward" => {
                        this.progress =
                            (this.progress + if id == "back" { -0.1 } else { 0.1 }).clamp(0., 1.);
                        this.playing = false;
                    }
                    _ => {}
                }
                this.last_frame = Instant::now();
                cx.notify();
            }))
    }
}

impl Render for Demo {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
        if self.playing
            && window.is_window_active()
            && self.curve.is_some()
            && window.supports_scene3d()
        {
            let next =
                self.progress + now.duration_since(self.last_frame).as_secs_f32().min(0.05) / 5.;
            if self.reveal {
                self.progress = next.min(1.);
                self.playing = self.progress < 1.;
            } else {
                self.progress = next.rem_euclid(1.);
            }
            if self.playing {
                window.request_animation_frame();
            }
        }
        self.last_frame = now;
        let mut scene = Scene::new()
            .camera(Camera::orbit(self.yaw, self.pitch, 5.4))
            .light(Light {
                direction: [-0.7, 1., 1.3],
                color: rgb(0xf3f8ff),
                intensity: 2.,
                ambient: 0.22,
            })
            .object(Object::new(
                self.cube.clone(),
                Material::color(rgb(0x527d94)),
            ));
        if let Some(curve) = &self.curve {
            let result = if self.reveal {
                curve.reveal(self.progress)
            } else {
                curve.object(self.progress)
            };
            match result {
                Ok(object) => {
                    scene = scene.object(object);
                    self.error = None;
                }
                Err(error) => {
                    self.error = Some(error.to_string());
                    self.playing = false;
                }
            }
        }
        let viewport = subtree_effect_chain(
            viewport3d("path", scene)
                .size_full()
                .color_samples(4)
                .resolution_scale(1.5),
            [EffectStage::bloom(BloomOptions {
                threshold: 0.6,
                radius: px(16.),
                intensity: 0.8,
                ..Default::default()
            })],
        );
        div()
            .size_full()
            .p_6()
            .flex()
            .flex_col()
            .gap_4()
            .bg(rgb(0x101824))
            .text_color(rgb(0xe3edf5))
            .child(div().text_size(px(28.)).child("Curve light"))
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0x94a9bb))
                    .child("Arc-length motion · Fixed width · Right-drag to rotate the view"),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_3()
                    .items_center()
                    .child(self.button(
                        "mode",
                        if self.reveal {
                            "Mode: Reveal"
                        } else {
                            "Mode: Flow"
                        },
                        cx,
                    ))
                    .child(self.button("play", if self.playing { "Pause" } else { "Play" }, cx))
                    .child(self.button("restart", "Restart", cx))
                    .child(self.button("back", "−10%", cx))
                    .child(self.button("forward", "+10%", cx))
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
                    .child(viewport),
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
                    size(px(1040.), px(720.)),
                    cx,
                ))),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| Demo::new(window, cx)),
        )
        .expect("curve light window");
    });
}
