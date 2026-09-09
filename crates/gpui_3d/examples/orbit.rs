use gpui::{
    App, Bounds, Context, Image, ImageFormat, ImageSource, MouseButton, Pixels, Point, Render,
    Window, WindowBounds, WindowOptions, div, prelude::*, px, rgb, size,
};
use gpui_3d::{Camera, Light, Material, Mesh, Object, Scene, viewport3d};
use gpui_platform::application;
use std::sync::Arc;

struct Orbit {
    yaw: f32,
    pitch: f32,
    distance: f32,
    drag: Option<Point<Pixels>>,
    texture: ImageSource,
    show_ui: bool,
}
impl Orbit {
    fn new() -> Self {
        Self {
            yaw: 0.35,
            pitch: 0.24,
            distance: 6.8,
            drag: None,
            show_ui: true,
            texture: Arc::new(Image::from_bytes(
                ImageFormat::Svg,
                include_bytes!("orbit.svg").to_vec(),
            ))
            .into(),
        }
    }
    fn ui(&self) -> impl IntoElement {
        div()
            .size_full()
            .p_12()
            .bg(rgb(0x20314b))
            .flex()
            .flex_col()
            .justify_between()
            .child(
                div()
                    .text_size(px(30.))
                    .text_color(rgb(0x8fdde5))
                    .child("GPUI / LIVE SURFACE"),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(
                        div()
                            .text_size(px(84.))
                            .text_color(rgb(0xf1f5ff))
                            .child("Hello, space."),
                    )
                    .child(
                        div()
                            .text_size(px(32.))
                            .text_color(rgb(0xb9c8df))
                            .child("文字、布局与图形，进入三维空间。"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_size(px(26.))
                            .text_color(rgb(0xa6b8d5))
                            .child("A real UI subtree"),
                    )
                    .child(
                        div()
                            .px_6()
                            .py_3()
                            .rounded_full()
                            .bg(rgb(0x91dbe2))
                            .text_color(rgb(0x172637))
                            .text_size(px(28.))
                            .child("EXPLORE"),
                    ),
            )
    }
}
impl Render for Orbit {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !window.is_window_active() {
            self.drag = None;
        }
        let mut scene = Scene::new()
            .camera(Camera::orbit(self.yaw, self.pitch, self.distance))
            .light(Light::default())
            .object(
                Object::new(Mesh::plane(), Material::color(rgb(0x28334a)))
                    .rotation([-std::f32::consts::FRAC_PI_2, 0., 0.])
                    .position([0., -1.1, 0.])
                    .scale([7., 5., 1.]),
            )
            .object(
                Object::new(Mesh::cube(), Material::image(self.texture.clone()))
                    .position([-1.35, -0.25, 0.])
                    .rotation([0., 0.4, 0.])
                    .scale([1.5; 3]),
            )
            .object(
                Object::new(
                    Mesh::plane(),
                    Material::image(self.texture.clone()).unlit(true),
                )
                .position([1.35, 0., -0.65])
                .rotation([0., -0.25, 0.])
                .scale([2.1, 1.6, 1.]),
            );
        if self.show_ui {
            scene = scene.object(
                Object::new(Mesh::plane(), Material::ui())
                    .position([0.4, 0.25, 0.85])
                    .scale([2.6, 1.6, 1.]),
            );
        }
        div()
            .size_full()
            .p_6()
            .bg(rgb(0x0c1422))
            .text_color(rgb(0xe6eefc))
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(div().text_size(px(30.)).child("A little more dimension"))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(0x96a9c6))
                                    .child("Drag to orbit · Scroll to zoom"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                div()
                                    .id("toggle-ui")
                                    .px_4()
                                    .py_2()
                                    .rounded_full()
                                    .bg(rgb(0x2f4360))
                                    .cursor_pointer()
                                    .child(if self.show_ui {
                                        "Hide UI plane"
                                    } else {
                                        "Show UI plane"
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.show_ui = !this.show_ui;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                div()
                                    .id("reset")
                                    .px_4()
                                    .py_2()
                                    .rounded_full()
                                    .bg(rgb(0x203047))
                                    .cursor_pointer()
                                    .child("Reset")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.yaw = 0.35;
                                        this.pitch = 0.24;
                                        this.distance = 6.8;
                                        this.drag = None;
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
            .child(
                div()
                    .id("orbit-area")
                    .relative()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .bg(rgb(0x141e30))
                    .rounded(px(24.))
                    .overflow_hidden()
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                            this.drag = Some(event.position);
                            cx.notify();
                        }),
                    )
                    .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                        if let Some(previous) = this.drag {
                            if event.pressed_button == Some(MouseButton::Left) {
                                let delta = event.position - previous;
                                this.yaw -= f32::from(delta.x) * 0.008;
                                this.pitch =
                                    (this.pitch + f32::from(delta.y) * 0.008).clamp(-1.3, 1.3);
                                this.drag = Some(event.position);
                            } else {
                                this.drag = None;
                            }
                            cx.notify();
                        }
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _, _, _| this.drag = None),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|this, _, _, _| this.drag = None),
                    )
                    .on_hover(cx.listener(|this, hovered: &bool, _, _| {
                        if !hovered {
                            this.drag = None;
                        }
                    }))
                    .on_scroll_wheel(cx.listener(|this, event: &gpui::ScrollWheelEvent, _, cx| {
                        let delta = event.delta.pixel_delta(px(20.));
                        this.distance =
                            (this.distance * (f32::from(delta.y) * 0.002).exp()).clamp(3., 14.);
                        cx.notify();
                    }))
                    .child(viewport3d("world", scene).ui_texture(self.ui()).size_full()),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0x96a9c6))
                    .child("Indexed meshes · Perspective · Depth testing · UI texture"),
            )
            .when(!window.supports_scene3d(), |root| {
                root.child("3D viewports are unavailable on this renderer.")
            })
    }
}
fn main() {
    application().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1160.), px(800.)),
                    cx,
                ))),
                ..Default::default()
            },
            |_, cx| cx.new(|_| Orbit::new()),
        )
        .expect("failed to open 3D example");
    });
}
