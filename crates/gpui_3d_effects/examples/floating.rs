use std::time::{Duration, Instant};

use gpui::{
    App, Bounds, Context, Render, Subscription, Window, WindowBounds, WindowOptions, div,
    prelude::*, px, rgb, rgba, size,
};
use gpui_3d::{
    Camera, Light, Material, Mesh, Object, PbrMaterial, Scene, SphereOptions, WgpuContext,
    viewport3d,
};
use gpui_3d_effects::{FloatingMotion, LightSweep, OrbitLight};
use gpui_effects::{BloomOptions, EffectStage, subtree_effect_chain};
use gpui_platform::application;

struct Demo {
    shapes: [Mesh; 2],
    shape: usize,
    motion: FloatingMotion,
    orbit: OrbitLight,
    sweep: Option<LightSweep>,
    sweep_error: Option<String>,
    sweep_on: bool,
    elapsed: Duration,
    last_frame: Instant,
    paused: bool,
    light_on: bool,
    glow: bool,
    _activation: Subscription,
}

impl Demo {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (sweep, sweep_error) = match WgpuContext::for_window(window).map(LightSweep::new) {
            Some(Ok(sweep)) => (
                Some(sweep.direction([1., 0.35, 0.]).range([-0.9, 0.9])),
                None,
            ),
            Some(Err(error)) => (None, Some(error.to_string())),
            None => (None, None),
        };
        Self {
            shapes: [
                Mesh::cube(),
                Mesh::sphere(SphereOptions {
                    radius: 0.72,
                    segments: [64, 32],
                })
                .unwrap(),
            ],
            shape: 0,
            motion: FloatingMotion::default(),
            orbit: OrbitLight::default().color(rgb(0x98e5f2)),
            sweep,
            sweep_error,
            sweep_on: true,
            elapsed: Duration::ZERO,
            last_frame: Instant::now(),
            paused: false,
            light_on: false,
            glow: true,
            _activation: cx.observe_window_activation(window, |this, _, cx| {
                this.last_frame = Instant::now();
                cx.notify();
            }),
        }
    }

    fn button(
        &self,
        id: &'static str,
        label: &'static str,
        active: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id(id)
            .px_4()
            .py_3()
            .rounded_lg()
            .bg(if active { rgb(0x34556a) } else { rgb(0x202d3e) })
            .hover(|style| style.bg(rgb(0x426d80)))
            .cursor_pointer()
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                match id {
                    "pause" => {
                        this.paused = !this.paused;
                        this.last_frame = Instant::now();
                    }
                    "shape" => this.shape = 1 - this.shape,
                    "light" => this.light_on = !this.light_on,
                    "glow" => this.glow = !this.glow,
                    "sweep" => this.sweep_on = !this.sweep_on,
                    _ => {}
                }
                cx.notify();
            }))
    }
}

impl Render for Demo {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
        if !self.paused && window.is_window_active() && window.supports_scene3d() {
            self.elapsed = self.elapsed.saturating_add(
                now.duration_since(self.last_frame)
                    .min(Duration::from_millis(50)),
            );
            window.request_animation_frame();
        }
        self.last_frame = now;
        let pose = self.motion.sample(self.elapsed);
        let mut material = Material::color(rgb(0x527d94)).pbr(PbrMaterial {
            metallic: 0.2,
            roughness: 0.32,
            ..Default::default()
        });
        if self.sweep_on
            && let Some(sweep) = &self.sweep
        {
            let progress = ((self.elapsed.as_secs_f64() % 3.6) / 2.8).min(1.) as f32;
            match sweep.pass(progress) {
                Ok(pass) => {
                    material = material.mesh_passes([pass]);
                    self.sweep_error = None;
                }
                Err(error) => self.sweep_error = Some(error.to_string()),
            }
        }
        let mut scene = Scene::new()
            .background(None)
            .camera(Camera::orbit(0.25, 0.15, 4.8))
            .light(Light {
                direction: [-0.7, 1., 1.3],
                color: rgb(0xf3f8ff),
                intensity: 2.5,
                ambient: 0.22,
            })
            .object(Object::new(self.shapes[self.shape].clone(), material).transform(pose));
        if self.light_on {
            scene = scene.object(
                self.orbit
                    .object(self.elapsed.as_secs_f32() * 0.9)
                    .scale([1.2; 3])
                    .position(pose.position),
            );
        }
        let spatial = subtree_effect_chain(
            viewport3d("spatial", scene)
                .size_full()
                .color_samples(4)
                .resolution_scale(1.5),
            [EffectStage::bloom(BloomOptions {
                threshold: if self.light_on { 0.55 } else { 0.75 },
                radius: px(if self.light_on { 24. } else { 12. }),
                intensity: if self.light_on { 1.3 } else { 0.35 },
                ..Default::default()
            })
            .enabled(self.glow)],
        );
        let controls = div()
            .w(px(240.))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().text_lg().child("2D controls"))
            .child(self.button(
                "pause",
                if self.paused { "Resume" } else { "Pause" },
                self.paused,
                cx,
            ))
            .child(self.button(
                "shape",
                if self.shape == 0 {
                    "Use sphere"
                } else {
                    "Use cube"
                },
                false,
                cx,
            ))
            .child(self.button("light", "Orbit light", self.light_on, cx))
            .child(self.button("sweep", "Light sweep", self.sweep_on, cx))
            .child(self.button("glow", "Glow", self.glow, cx));
        div()
            .size_full()
            .bg(rgb(0x101824))
            .text_color(rgb(0xdce8ee))
            .p_6()
            .flex()
            .flex_col()
            .gap_5()
            .child(div().text_size(px(26.)).child("Spatial effects"))
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0x8fa7b8))
                    .child("Floating motion · Surface light sweep · Orbit light"),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .items_center()
                    .gap_6()
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .relative()
                            .rounded_xl()
                            .bg(rgba(0xffffff04))
                            .child(spatial)
                            .child(
                                div()
                                    .absolute()
                                    .left_4()
                                    .bottom_4()
                                    .px_3()
                                    .py_2()
                                    .rounded_lg()
                                    .bg(rgba(0x101824cc))
                                    .text_sm()
                                    .child("2D overlay"),
                            ),
                    )
                    .child(controls),
            )
            .when(!window.supports_scene3d(), |root| {
                root.child("3D rendering is unavailable on this window.")
            })
            .when_some(self.sweep_error.clone(), |root, error| {
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
                    size(px(960.), px(620.)),
                    cx,
                ))),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| Demo::new(window, cx)),
        )
        .expect("spatial effects window");
    });
}
