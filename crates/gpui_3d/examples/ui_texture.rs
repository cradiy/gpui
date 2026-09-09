use gpui::{
    App, Bounds, Context, MouseButton, Pixels, Point, Render, Window, WindowBounds, WindowOptions,
    div, prelude::*, px, rgb, size,
};
use gpui_3d::{Camera, Material, Mesh, Object, Scene, viewport3d};
use gpui_platform::application;

struct UiTexture {
    density: f32,
    width: f32,
    yaw: f32,
    pitch: f32,
    distance: f32,
    drag: Option<Point<Pixels>>,
    _activation: gpui::Subscription,
}

impl UiTexture {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            density: 1.,
            width: 640.,
            yaw: 0.15,
            pitch: 0.08,
            distance: 4.,
            drag: None,
            _activation: cx.observe_window_activation(window, |this, window, _| {
                if !window.is_window_active() {
                    this.drag = None;
                }
            }),
        }
    }

    fn panel(&self) -> impl IntoElement {
        div()
            .size_full()
            .p_8()
            .rounded(px(28.))
            .bg(rgb(0x1d2e43))
            .text_color(rgb(0xeaf3fc))
            .flex()
            .flex_col()
            .justify_between()
            .child(
                div().flex().justify_between().items_center()
                    .child(div().text_size(px(14.)).text_color(rgb(0x8bd6dc)).child("SURFACE / 01"))
                    .child(div().size(px(44.)).rounded_full().bg(rgb(0x8bd6dc))),
            )
            .child(
                div().flex().flex_col().gap_3()
                    .child(div().text_size(px(44.)).child("Room to think."))
                    .child(div().text_size(px(18.)).text_color(rgb(0xb1c4d9))
                        .child("A fixed canvas, viewed from any angle. Resize the window without changing the layout.")),
            )
            .child(
                div().flex().flex_col().gap_3()
                    .child(div().flex().gap(px(1.)).children((0..100).map(|i| {
                        div().w(px(1.)).h(px(if i % 5 == 0 { 24. } else { 12. }))
                            .bg(rgb(0x8bd6dc)).flex_shrink_0()
                    })))
                    .child(div().flex().justify_between().text_size(px(13.))
                        .text_color(rgb(0xb1c4d9))
                        .child("Fine detail / 1 px lines")
                        .child(format!("{:.0} × 400 logical px", self.width))),
            )
    }
}

impl Render for UiTexture {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let config = gpui::UiTexture3d::new(
            size(px(self.width), px(400.)),
            window.scale_factor() * self.density,
        );
        let pixels = config.pixel_size();
        let scene = Scene::new()
            .camera(Camera::orbit(self.yaw, self.pitch, self.distance))
            .object(Object::new(Mesh::plane(), Material::ui()).scale([
                3.8,
                3.8 * 400. / self.width,
                1.,
            ]));
        div()
            .size_full()
            .p_6()
            .flex()
            .flex_col()
            .gap_4()
            .bg(rgb(0x0c1422))
            .text_color(rgb(0xeaf3fc))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().text_size(px(30.)).child("A canvas of its own"))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(0x91a8c3))
                            .child("Drag to orbit · Scroll to zoom · Resize the window"),
                    ),
            )
            .child(
                div()
                    .id("camera")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .rounded(px(24.))
                    .bg(rgb(0x121e30))
                    .overflow_hidden()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &gpui::MouseDownEvent, _, _| {
                            this.drag = Some(event.position);
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
                                cx.notify();
                            } else {
                                this.drag = None;
                            }
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
                        this.distance = (this.distance
                            * (f32::from(event.delta.pixel_delta(px(20.)).y) * 0.002).exp())
                        .clamp(2.5, 10.);
                        cx.notify();
                    }))
                    .child(
                        viewport3d("panel", scene)
                            .size_full()
                            .ui_texture(self.panel())
                            .ui_texture_size(size(px(self.width), px(400.)))
                            .ui_texture_scale(self.density),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_3()
                    .children(
                        [0.25, 1., 2.]
                            .into_iter()
                            .enumerate()
                            .map(|(index, density)| {
                                div()
                                    .id(("density", index))
                                    .px_4()
                                    .py_2()
                                    .rounded_full()
                                    .cursor_pointer()
                                    .bg(rgb(if self.density == density {
                                        0x315e76
                                    } else {
                                        0x203047
                                    }))
                                    .child(format!("{density}× density"))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.density = density;
                                        cx.notify();
                                    }))
                            }),
                    )
                    .child(
                        div()
                            .id("layout-width")
                            .px_4()
                            .py_2()
                            .rounded_full()
                            .bg(rgb(0x203047))
                            .cursor_pointer()
                            .child(format!("Layout · {:.0} × 400", self.width))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.width = if this.width == 640. { 960. } else { 640. };
                                cx.notify();
                            })),
                    )
                    .child(div().text_sm().text_color(rgb(0x91a8c3)).child(format!(
                        "{} × {} texture px",
                        pixels.width.0, pixels.height.0
                    ))),
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
                    size(px(1100.), px(800.)),
                    cx,
                ))),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| UiTexture::new(window, cx)),
        )
        .expect("failed to open UI texture example");
    });
}
