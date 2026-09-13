use std::{cell::Cell, rc::Rc, time::Instant};

use gpui::{
    App, Bounds, Context, Pixels, Render, Subscription, Window, WindowBounds, WindowOptions,
    canvas, div, prelude::*, px, rgb, size,
};
use gpui_3d::{Camera, Material, Mesh, Object, Scene, viewport3d};
use gpui_3d_effects::InertialTilt;
use gpui_platform::application;

struct Demo {
    tilt: InertialTilt,
    plane: Mesh,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    last_frame: Instant,
    springy: bool,
    _activation: Subscription,
}

impl Demo {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            tilt: InertialTilt::default(),
            plane: Mesh::plane(),
            bounds: Rc::new(Cell::new(Bounds::default())),
            last_frame: Instant::now(),
            springy: false,
            _activation: cx.observe_window_activation(window, |this, window, cx| {
                this.last_frame = Instant::now();
                if !window.is_window_active() {
                    this.tilt.set_target([0.; 2]);
                }
                cx.notify();
            }),
        }
    }

    fn retarget(&mut self, target: [f32; 2]) {
        let now = Instant::now();
        self.tilt.advance(now.duration_since(self.last_frame));
        self.last_frame = now;
        self.tilt.set_target(target);
    }

    fn surface(&self) -> impl IntoElement {
        div()
            .size_full()
            .rounded(px(24.))
            .bg(rgb(0xf0f5fa))
            .text_color(rgb(0x20364a))
            .p_8()
            .flex()
            .flex_col()
            .gap_5()
            .child(div().text_sm().text_color(rgb(0x57748c)).child("MOTION"))
            .child(
                div()
                    .flex()
                    .gap_3()
                    .items_end()
                    .child(div().w(px(40.)).h(px(56.)).rounded_lg().bg(rgb(0x7bc6d0)))
                    .child(div().w(px(40.)).h(px(88.)).rounded_lg().bg(rgb(0x6589b0)))
                    .child(div().w(px(40.)).h(px(72.)).rounded_lg().bg(rgb(0xc2b2d8))),
            )
            .child(div().text_size(px(34.)).child("A lighter touch"))
            .child(div().text_base().child("Direction with momentum."))
            .child(div().h(px(8.)).w_full().rounded_full().bg(rgb(0xd5e0ea)))
            .child(div().h(px(8.)).w(px(164.)).rounded_full().bg(rgb(0xd5e0ea)))
            .child(div().flex_1())
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0x57748c))
                    .child("Move · Release · Rest"),
            )
    }
}

impl Render for Demo {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
        if window.is_window_active() && window.supports_scene3d() {
            self.tilt.advance(now.duration_since(self.last_frame));
            if !self.tilt.is_settled() {
                window.request_animation_frame();
            }
        }
        self.last_frame = now;
        let pose = self.tilt.pose();
        let scene = Scene::new().camera(Camera::orbit(0., 0., 5.2)).object(
            Object::new(self.plane.clone(), Material::ui())
                .transform(pose)
                .scale([1.65, 2.0625, 1.]),
        );
        let bounds = Rc::clone(&self.bounds);
        div()
            .size_full()
            .p_6()
            .flex()
            .flex_col()
            .gap_4()
            .bg(rgb(0x101824))
            .text_color(rgb(0xe3edf5))
            .child(div().text_size(px(28.)).child("Inertial tilt"))
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0x94a9bb))
                    .child("Move across the surface · Leave to return to rest"),
            )
            .child(
                div()
                    .flex()
                    .gap_4()
                    .items_center()
                    .child(
                        div()
                            .id("damping")
                            .px_4()
                            .py_2()
                            .rounded_lg()
                            .bg(rgb(0x263b50))
                            .hover(|style| style.bg(rgb(0x36536c)))
                            .cursor_pointer()
                            .child(if self.springy { "Springy" } else { "Soft" })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.springy = !this.springy;
                                this.tilt =
                                    this.tilt.damping(if this.springy { 0.4 } else { 0.75 });
                                cx.notify();
                            })),
                    )
                    .child(format!(
                        "X {:+.1}° · Y {:+.1}°",
                        pose.rotation[0].to_degrees(),
                        pose.rotation[1].to_degrees()
                    )),
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
                    .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                        let bounds = this.bounds.get();
                        if bounds.size.width > px(0.) && bounds.size.height > px(0.) {
                            let local = event.position - bounds.origin;
                            let x = (f32::from(local.x) / f32::from(bounds.size.width) * 2. - 1.)
                                .clamp(-1., 1.);
                            let y = (f32::from(local.y) / f32::from(bounds.size.height) * 2. - 1.)
                                .clamp(-1., 1.);
                            this.retarget([y * 0.3, x * 0.3]);
                            cx.notify();
                        }
                    }))
                    .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                        if !hovered {
                            this.retarget([0.; 2]);
                            cx.notify();
                        }
                    }))
                    .child(
                        canvas(move |region, _, _| bounds.set(region), |_, _, _, _| {})
                            .absolute()
                            .inset_0()
                            .size_full(),
                    )
                    .child(
                        viewport3d("tilt", scene)
                            .size_full()
                            .color_samples(4)
                            .ui_texture_size(size(px(352.), px(440.)))
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
        .expect("inertial tilt window");
    });
}
