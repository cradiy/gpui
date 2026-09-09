use gpui::{
    App, Bounds, Context, MouseButton, Pixels, Render, Window, WindowBounds, WindowOptions, canvas,
    div, prelude::*, px, rgb, size,
};
use gpui_3d::{
    Camera, DirectionalShadow, Light, Material, Mesh, Object, OrbitController, PbrMaterial, Scene,
    viewport3d,
};
use gpui_platform::application;
use std::{cell::Cell, rc::Rc};

struct ShadowsDemo {
    enabled: bool,
    soft: bool,
    resolution: u32,
    sun: [f32; 3],
    height: f32,
    controls: OrbitController,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    _activation: gpui::Subscription,
}

impl ShadowsDemo {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
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
        let material = |color| {
            Material::color(rgb(color)).pbr(PbrMaterial {
                metallic: 0.,
                roughness: 0.7,
                ..Default::default()
            })
        };
        Scene::new()
            .camera(self.controls.camera())
            .light(Light {
                direction: self.sun,
                color: rgb(0xfff2dc),
                intensity: 2.4,
                ambient: 0.12,
            })
            .directional_shadow(self.enabled.then_some(DirectionalShadow {
                resolution: self.resolution,
                softness: if self.soft { 1.5 } else { 0. },
                ..DirectionalShadow::new([0., 0.3, 0.], [3.6, 3.6, 5.])
            }))
            .object(
                Object::new(Mesh::plane(), material(0xbdc8d5))
                    .rotation([-std::f32::consts::FRAC_PI_2, 0., 0.])
                    .position([0., -0.6, 0.])
                    .scale([5., 5., 1.]),
            )
            .object(
                Object::new(Mesh::cube(), material(0xdca678))
                    .position([-0.8, -0.1 + self.height, 0.3])
                    .scale([0.9, 1., 0.9]),
            )
            .object(
                Object::new(Mesh::cube(), material(0x71b8c0))
                    .position([0.7, 0.3 + self.height, -0.45])
                    .rotation([0., 0.35, 0.])
                    .scale([0.6, 1.8, 0.6]),
            )
            .object(
                Object::new(Mesh::cube(), material(0xa299d4))
                    .position([0.65, -0.4, 1.])
                    .scale([0.8, 0.4, 0.6]),
            )
    }
}

impl Render for ShadowsDemo {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bounds = self.bounds.clone();
        let view_id = cx.entity_id();
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
                    .child("Move pointer to steer sunlight · Right-drag to orbit · Scroll to zoom"),
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
                    .child(viewport3d("shadow-scene", self.scene()).size_full())
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
            |window, cx| cx.new(|cx| ShadowsDemo::new(window, cx)),
        )
        .expect("failed to open shadows example");
    });
}
