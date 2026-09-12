use gpui::{
    App, Bounds, Context, MouseButton, Pixels, Render, Window, WindowBounds, WindowOptions, canvas,
    div, prelude::*, px, rgb, size,
};
use gpui_3d::{
    Camera, DiffuseEnvironment, DirectionalShadow, EnvironmentBackground, EnvironmentMap, Light,
    Material, Mesh, Object, OrbitController, PbrMaterial, PunctualLight, Scene,
    SpecularEnvironment, SpecularPrefilter, viewport3d,
};
use gpui_effects::{BloomOptions, EffectStage, SubtreeColorOptions, subtree_effect_chain};
use gpui_platform::application;
use std::{cell::Cell, rc::Rc};

fn environment() -> EnvironmentMap {
    let pixels: Vec<_> = (0..64)
        .flat_map(|y| {
            (0..128).map(move |x| {
                let theta = (y as f32 + 0.5) * std::f32::consts::PI / 64.;
                let phi = (x as f32 + 0.5) * std::f32::consts::TAU / 128. - std::f32::consts::PI;
                let direction = [
                    theta.sin() * phi.cos(),
                    theta.cos(),
                    theta.sin() * phi.sin(),
                ];
                let sky = direction[1].max(0.);
                let ground = (-direction[1]).max(0.);
                let warm = (direction[0] * 0.8 + direction[2] * 0.6).max(0.).powi(6) * 5.;
                [
                    0.08 + 0.1 * sky + 0.4 * ground + warm,
                    0.08 + 0.5 * sky + 0.15 * ground + warm * 0.4,
                    0.08 + 1.5 * sky + 0.05 * ground + warm * 0.08,
                ]
            })
        })
        .collect();
    EnvironmentMap::from_equirectangular([128, 64], pixels).unwrap()
}

struct Lighting {
    bloom: bool,
    grading: usize,
    kind: usize,
    fill: bool,
    environment: DiffuseEnvironment,
    specular: SpecularEnvironment,
    specular_on: bool,
    specular_rotation: f32,
    roughness: f32,
    background: EnvironmentBackground,
    background_on: bool,
    background_rotation: f32,
    background_intensity: f32,
    environment_on: bool,
    environment_rotation: f32,
    cone: f32,
    range: f32,
    enabled: bool,
    soft: bool,
    resolution: u32,
    sun: [f32; 3],
    height: f32,
    controls: OrbitController,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    _activation: gpui::Subscription,
}

impl Lighting {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let map = environment();
        Self {
            bloom: false,
            grading: 0,
            kind: 0,
            fill: false,
            environment: DiffuseEnvironment::from_map(&map).unwrap(),
            specular: SpecularEnvironment::from_map(&map, SpecularPrefilter::default()).unwrap(),
            specular_on: true,
            specular_rotation: 0.,
            roughness: 0.15,
            background: EnvironmentBackground::new(map),
            background_on: true,
            background_rotation: 0.,
            background_intensity: 0.35,
            environment_on: false,
            environment_rotation: 0.,
            cone: 0.6,
            range: 6.,
            enabled: true,
            soft: true,
            resolution: 2048,
            sun: [-1., 1.8, 1.],
            height: 0.,
            controls: OrbitController::new(Camera::orbit(0.5, 0.55, 6.)).unwrap(),
            bounds: Rc::new(Cell::new(Bounds::default())),
            _activation: cx.observe_window_activation(window, |this, window, _| {
                if !window.is_window_active() {
                    this.controls.cancel_drag();
                }
            }),
        }
    }

    fn scene(&self) -> Scene {
        let material = |color, metallic, roughness| {
            Material::color(rgb(color)).pbr(PbrMaterial {
                metallic,
                roughness,
                ..Default::default()
            })
        };
        let position = [self.sun[0], 2.8, self.sun[2]];
        let source = match self.kind {
            0 => PunctualLight::directional(self.sun).intensity(2.4),
            1 => PunctualLight::point(position)
                .intensity(18.)
                .range(Some(self.range)),
            _ => PunctualLight::spot(position, position.map(|v| -v))
                .cone_angles(self.cone * 0.5, self.cone)
                .intensity(18.)
                .range(Some(self.range)),
        }
        .color(rgb(0xfff2dc));
        let mut lights = vec![source];
        if self.fill {
            lights.push(
                PunctualLight::directional([1., 0.5, -1.])
                    .color(rgb(0x719eff))
                    .intensity(0.7),
            );
        }
        Scene::new()
            .camera(self.controls.camera())
            .light(Light {
                direction: self.sun,
                color: rgb(0xfff2dc),
                intensity: 2.4,
                ambient: 0.12,
            })
            .lights(lights)
            .specular_environment(
                self.specular_on
                    .then(|| self.specular.clone().rotation_y(self.specular_rotation)),
            )
            .background(self.background_on.then(|| {
                self.background
                    .clone()
                    .intensity(self.background_intensity)
                    .rotation_y(self.background_rotation)
            }))
            .diffuse_environment(
                self.environment
                    .rotation_y(self.environment_rotation)
                    .intensity(if self.environment_on { 0.35 } else { 0. }),
            )
            .directional_shadow(
                (self.enabled && self.kind == 0).then_some(DirectionalShadow {
                    resolution: self.resolution,
                    softness: if self.soft { 1.5 } else { 0. },
                    ..DirectionalShadow::new([0., 0.3, 0.], [3.6, 3.6, 5.])
                }),
            )
            .object(
                Object::new(Mesh::plane(), material(0xbdc8d5, 0., 0.7))
                    .rotation([-std::f32::consts::FRAC_PI_2, 0., 0.])
                    .position([0., -0.6, 0.])
                    .scale([5., 5., 1.]),
            )
            .object(
                Object::new(Mesh::cube(), material(0xdca678, 0.9, self.roughness))
                    .position([-0.8, -0.1 + self.height, 0.3])
                    .scale([0.9, 1., 0.9]),
            )
            .object(
                Object::new(Mesh::cube(), material(0x71b8c0, 0., self.roughness))
                    .position([0.7, 0.3 + self.height, -0.45])
                    .rotation([0., 0.35, 0.])
                    .scale([0.6, 1.8, 0.6]),
            )
            .object(
                Object::new(Mesh::cube(), material(0xa299d4, 1., self.roughness))
                    .position([0.65, -0.4, 1.])
                    .scale([0.8, 0.4, 0.6]),
            )
    }
}

impl Render for Lighting {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bounds = self.bounds.clone();
        let view_id = cx.entity_id();
        let effects_supported = window.supports_subtree_effects();
        let grading = match self.grading {
            1 => SubtreeColorOptions {
                saturation: 0.,
                ..Default::default()
            },
            2 => SubtreeColorOptions {
                saturation: 1.3,
                contrast: 1.1,
                brightness: 1.,
            },
            _ => SubtreeColorOptions::default(),
        };
        let viewport = subtree_effect_chain(
            viewport3d("shadow-scene", self.scene()).size_full(),
            [
                EffectStage::bloom(BloomOptions {
                    threshold: 0.7,
                    soft_knee: 0.1,
                    intensity: 1.4,
                    radius: px(32.),
                    downsample: 2,
                })
                .enabled(self.bloom),
                EffectStage::color_adjust(grading).enabled(self.grading != 0),
            ],
        )
        .map_interaction(true)
        .enabled(effects_supported);
        div()
            .size_full()
            .p_6()
            .flex()
            .flex_col()
            .gap_4()
            .bg(rgb(0x0c1421))
            .text_color(rgb(0xe8f0fa))
            .child(div().text_size(px(30.)).child("Light and shadow"))
            .child(
                div()
                    .text_color(rgb(0x95aac5))
                    .child("Move pointer to move the light · Right-drag to orbit · Scroll to zoom"),
            )
            .child(
                div().flex().flex_wrap().gap_3().children(
                    [
                        (
                            "shadow",
                            if self.enabled {
                                "Shadows on"
                            } else {
                                "Shadows off"
                            },
                        ),
                        (
                            "soft",
                            if self.soft {
                                "Soft edges"
                            } else {
                                "Hard edges"
                            },
                        ),
                        ("resolution", "Change resolution"),
                        (
                            "lift",
                            if self.height == 0. {
                                "Lift objects"
                            } else {
                                "Lower objects"
                            },
                        ),
                        ("source", "Light type"),
                        ("fill", "Fill light"),
                        ("environment", "Environment"),
                        ("rotate", "Rotate environment"),
                        ("background", if self.background_on { "Hide background" } else { "Show background" }),
                        ("background-rotate", "Rotate background"),
                        ("background-brightness", "Background brightness"),
                        ("specular", if self.specular_on { "Reflections on" } else { "Reflections off" }),
                        ("specular-rotate", "Rotate reflections"),
                        ("roughness", "Roughness"),
                        ("cone", "Spot cone"),
                        ("range", "Light range"),
                        ("bloom", if self.bloom { "Bloom on" } else { "Bloom off" }),
                        ("grading", ["Color: Natural", "Color: Monochrome", "Color: Vivid"][self.grading]),
                        ("reset", "Reset view"),
                    ]
                    .into_iter()
                    .map(|(id, label)| {
                        div()
                            .id(id)
                            .px_4()
                            .py_2()
                            .rounded(px(10.))
                            .bg(rgb(0x263d56))
                            .cursor_pointer()
                            .hover(|s| s.bg(rgb(0x365974)))
                            .child(label)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                match id {
                                    "bloom" => this.bloom = !this.bloom,
                                    "grading" => this.grading = (this.grading + 1) % 3,
                                    "source" => this.kind = (this.kind+1)%3,
                                    "fill" => this.fill = !this.fill,
                                    "environment" => this.environment_on = !this.environment_on,
                                    "rotate" => this.environment_rotation += 0.4,
                                    "background" => this.background_on = !this.background_on,
                                    "background-rotate" => this.background_rotation += 0.4,
                                    "background-brightness" => this.background_intensity = if this.background_intensity < 0.8 { this.background_intensity + 0.2 } else { 0.15 },
                                    "specular" => this.specular_on = !this.specular_on,
                                    "specular-rotate" => this.specular_rotation += 0.4,
                                    "roughness" => this.roughness = if this.roughness < 0.95 { (this.roughness + 0.2).min(1.) } else { 0.05 },
                                    "cone" => this.cone = if this.cone < 1. { this.cone+0.2 } else { 0.3 },
                                    "range" => this.range = if this.range < 8. { this.range+2. } else { 4. },
                                    "shadow" => this.enabled = !this.enabled,
                                    "soft" => this.soft = !this.soft,
                                    "resolution" => {
                                        this.resolution = match this.resolution {
                                            512 => 1024,
                                            1024 => 2048,
                                            2048 => 4096,
                                            _ => 512,
                                        }
                                    }
                                    "lift" => {
                                        this.height = if this.height == 0. { 0.6 } else { 0. }
                                    }
                                    _ => this
                                        .controls
                                        .set_camera(Camera::orbit(0.5, 0.55, 6.))
                                        .unwrap(),
                                }
                                cx.notify();
                            }))
                    }),
                ),
            )
            .child(
                div()
                    .id("shadow-stage")
                    .relative()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .rounded(px(20.))
                    .overflow_hidden()
                    .bg(rgb(0x142237))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                            if this
                                .controls
                                .begin_drag(event.button, event.position, this.bounds.get())
                                .unwrap_or(false)
                            {
                                cx.stop_propagation();
                            }
                        }),
                    )
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
                        } else if event.pressed_button.is_none() {
                            let rect = this.bounds.get();
                            if rect.size.width > px(0.) && rect.size.height > px(0.) {
                                let x = (event.position.x - rect.origin.x) / rect.size.width;
                                let y = (event.position.y - rect.origin.y) / rect.size.height;
                                this.sun = [(x - 0.5) * 4., 1.8, (y - 0.5) * 3.];
                                cx.notify();
                            }
                        }
                    }))
                    .on_mouse_up(
                        MouseButton::Right,
                        cx.listener(|this, _, _, _| {
                            this.controls.end_drag(MouseButton::Right);
                        }),
                    )
                    .on_mouse_up_out(
                        MouseButton::Right,
                        cx.listener(|this, _, _, _| {
                            this.controls.end_drag(MouseButton::Right);
                        }),
                    )
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
                    ),
            )
            .when(!effects_supported, |root| root.child(
                div().text_color(rgb(0xa8bdd6)).child("Subtree effects are unavailable on this renderer."),
            ))
            .child(div().text_color(rgb(0xa8bdd6)).child(format!("{} · Fill {} · Environment {} · Cone {:.0}° · Range {:.0} · Shadows available for directional light",
                ["Directional", "Point", "Spot"][self.kind], self.fill, self.environment_on, self.cone.to_degrees(), self.range)))
            .child(div().text_color(rgb(0xa8bdd6)).child(format!(
                "Shadow map {} × {} · {} · Ambient light stays visible in shadow",
                self.resolution,
                self.resolution,
                if self.soft {
                    "PCF filtering"
                } else {
                    "Hard comparison"
                }
            )))
    }
}

fn main() {
    application().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1200.), px(820.)),
                    cx,
                ))),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| Lighting::new(window, cx)),
        )
        .expect("failed to open lighting example");
    });
}
