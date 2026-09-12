use gpui::{
    App, Bounds, Context, Entity, MouseButton, Pixels, Point, Render, ScrollHandle, Subscription,
    Window, WindowBounds, WindowOptions, div, prelude::*, px, rgb, size,
};
use gpui_3d::{Camera, Material, Mesh, Object, PickBehavior, Scene, viewport3d};
use gpui_platform::application;
use uic::components::slider::{Slider, SliderState};

struct UiDemo {
    level: Entity<SliderState>,
    left: usize,
    right: usize,
    covered: bool,
    density: f32,
    width: f32,
    yaw: f32,
    pitch: f32,
    distance: f32,
    drag: Option<(MouseButton, Point<Pixels>)>,
    scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl UiDemo {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let level = cx.new(|cx| SliderState::new(35., 0. ..=100., cx));
        let changed = cx.subscribe(&level, |_, _, _, cx| cx.notify());
        let activation = cx.observe_window_activation(window, |this, window, _| {
            if !window.is_window_active() {
                this.drag = None;
            }
        });
        Self {
            level,
            left: 0,
            right: 0,
            covered: false,
            density: 1.,
            width: 640.,
            yaw: 0.25,
            pitch: 0.12,
            distance: 4.8,
            drag: None,
            scroll: ScrollHandle::new(),
            _subscriptions: vec![changed, activation],
        }
    }

    fn panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let value = self.level.read(cx).value();
        div()
            .size_full()
            .p_8()
            .rounded(px(24.))
            .bg(rgb(0x1b2d44))
            .text_color(rgb(0xeaf3fc))
            .flex()
            .flex_col()
            .gap_5()
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(rgb(0x8ed6df))
                    .child("POINTER / SURFACE"),
            )
            .child(div().text_size(px(32.)).child("Touch what you see."))
            .child(
                div()
                    .flex()
                    .gap_4()
                    .children(
                        [(true, self.left), (false, self.right)].map(|(left, count)| {
                            div()
                                .id(if left { "left" } else { "right" })
                                .flex_1()
                                .py_4()
                                .rounded(px(12.))
                                .bg(rgb(0x324d6a))
                                .hover(|style| style.bg(rgb(0x47718d)))
                                .cursor_pointer()
                                .flex()
                                .justify_center()
                                .child(format!("{} · {count}", if left { "Left" } else { "Right" }))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if left {
                                        this.left += 1;
                                    } else {
                                        this.right += 1;
                                    }
                                    cx.notify();
                                }))
                        }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .text_sm()
                            .child("LEVEL")
                            .child(format!("{value:.0}%")),
                    )
                    .child(Slider::new(&self.level).label("Level"))
                    .child(
                        div().h(px(6.)).rounded_full().bg(rgb(0x30445c)).child(
                            div()
                                .h_full()
                                .w(gpui::relative(value as f32 / 100.))
                                .rounded_full()
                                .bg(rgb(0x8ed6df)),
                        ),
                    ),
            )
            .child(
                div()
                    .id("notes")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .rounded(px(12.))
                    .bg(rgb(0x132237))
                    .child(
                        div()
                            .p_4()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .children((1..=16).map(|index| {
                                div()
                                    .text_size(px(15.))
                                    .text_color(rgb(0xb6cadf))
                                    .child(format!("{index:02} / A note on this surface"))
                            })),
                    ),
            )
    }
}

impl Render for UiDemo {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut scene = Scene::new()
            .camera(Camera::orbit(self.yaw, self.pitch, self.distance))
            .object(
                Object::new(Mesh::plane(), Material::ui())
                    .id("panel")
                    .scale([4., 4. * 520. / self.width, 1.]),
            );
        if self.covered {
            scene = scene.object(
                Object::new(Mesh::plane(), Material::color(rgb(0x476174)).unlit(true))
                    .id("cover")
                    .pick_behavior(PickBehavior::Occlude)
                    .position([-1.05, 0., 0.3])
                    .scale([1.7, 3.5, 1.]),
            );
        }
        div()
            .size_full()
            .p_6()
            .flex()
            .flex_col()
            .gap_4()
            .bg(rgb(0x0b1422))
            .text_color(rgb(0xeaf3fc))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_between()
                    .items_center()
                    .gap_4()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(div().text_size(px(28.)).child("UI, in perspective"))
                            .child(div().text_sm().text_color(rgb(0x92a9c3)).child(
                                "Click and drag controls · Scroll the notes · Right-drag to orbit",
                            )),
                    )
                    .child(
                        div()
                            .id("cover-toggle")
                            .px_4()
                            .py_2()
                            .rounded_full()
                            .bg(rgb(0x29415d))
                            .cursor_pointer()
                            .child(if self.covered {
                                "Remove occluder"
                            } else {
                                "Add occluder"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.covered = !this.covered;
                                cx.notify();
                            })),
                    ),
            )
            .child(div().flex().flex_wrap().gap_3().children([
                ("density", "Raster density"), ("width", "Canvas width")
            ].into_iter().map(|(id, label)| div().id(id).px_4().py_2().rounded_full()
                .bg(rgb(0x29415d)).cursor_pointer().child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if id == "density" { this.density = if this.density < 2. { this.density * 2. } else { 0.5 }; }
                    else { this.width = if this.width == 640. { 800. } else { 640. }; }
                    cx.notify();
                })))))
            .child(
                div()
                    .id("camera")
                    .relative()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .rounded(px(24.))
                    .overflow_hidden()
                    .bg(rgb(0x111e30))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &gpui::MouseDownEvent, _, _| {
                            this.drag = Some((event.button, event.position));
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, event: &gpui::MouseDownEvent, _, _| {
                            this.drag = Some((event.button, event.position));
                        }),
                    )
                    .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                        if let Some((button, previous)) = this.drag {
                            if event.pressed_button == Some(button) {
                                let delta = event.position - previous;
                                this.yaw -= f32::from(delta.x) * 0.008;
                                this.pitch =
                                    (this.pitch + f32::from(delta.y) * 0.008).clamp(-1.3, 1.3);
                                this.drag = Some((button, event.position));
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
                    .on_mouse_up(
                        MouseButton::Right,
                        cx.listener(|this, _, _, _| this.drag = None),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
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
                    .on_scroll_wheel(cx.listener(|this, event: &gpui::ScrollWheelEvent, _, cx| {
                        this.distance = (this.distance
                            * (f32::from(event.delta.pixel_delta(px(20.)).y) * 0.002).exp())
                        .clamp(3., 10.);
                        cx.notify();
                    }))
                    .child(
                        viewport3d("world", scene)
                            .size_full()
                            .ui_texture_size(size(px(self.width), px(520.)))
                            .ui_texture_scale(self.density)
                            .ui_texture(self.panel(cx))
                            .interactive_ui("panel"),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0x92a9c3))
                    .child(format!("Canvas {:.0} × 520 · Raster {:.1}× · Left-drag empty space to orbit · Scroll outside the panel to zoom", self.width, self.density)),
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
                    size(px(1160.), px(860.)),
                    cx,
                ))),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| UiDemo::new(window, cx)),
        )
        .expect("failed to open UI example");
    });
}
