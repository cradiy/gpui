use gpui::{
    App, Bounds, Context, DispatchPhase, HitboxBehavior, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Render, Window, WindowBounds, WindowOptions, canvas, div, point,
    prelude::*, px, relative, rgb, size,
};
use gpui_effects::{DeformationOptions, EffectStage, LensOptions, subtree_effect_chain};
use gpui_platform::application;

struct InteractionPreview {
    lens: bool,
    mapped: bool,
    clicks: [usize; 2],
    value: f32,
    direction: f32,
    dragging: bool,
}

impl InteractionPreview {
    fn control(
        &self,
        index: usize,
        label: &'static str,
        active: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id(("control", index))
            .px_4()
            .py_2()
            .rounded_full()
            .cursor_pointer()
            .bg(rgb(if active { 0x315d79 } else { 0x202d43 }))
            .hover(|s| s.bg(rgb(0x416a85)))
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                match index {
                    0 => this.lens = true,
                    1 => this.lens = false,
                    2 => this.mapped = !this.mapped,
                    _ => this.direction = -this.direction,
                }
                cx.notify();
            }))
    }

    fn slider(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity().downgrade();
        div()
            .relative()
            .w_full()
            .h(px(40.))
            .cursor_pointer()
            .child(
                div()
                    .absolute()
                    .top(px(17.))
                    .w_full()
                    .h(px(6.))
                    .rounded_full()
                    .bg(rgb(0x314259)),
            )
            .child(
                div()
                    .absolute()
                    .top(px(17.))
                    .w(relative(self.value))
                    .h(px(6.))
                    .rounded_full()
                    .bg(rgb(0x86d5e8)),
            )
            .child(
                div()
                    .absolute()
                    .left(relative(self.value))
                    .ml(px(-10.))
                    .top(px(10.))
                    .size(px(20.))
                    .rounded_full()
                    .bg(rgb(0xc3f0f6)),
            )
            .child(
                canvas(
                    |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
                    move |bounds, hitbox, window, _| {
                        let down_entity = entity.clone();
                        let down_hitbox = hitbox;
                        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                            if phase == DispatchPhase::Bubble
                                && event.button == MouseButton::Left
                                && down_hitbox.is_hovered(window)
                            {
                                window.capture_pointer(down_hitbox.id);
                                let _ = down_entity.update(cx, |this, cx| {
                                    this.dragging = true;
                                    this.value = (f32::from(event.position.x - bounds.left())
                                        / f32::from(bounds.size.width).max(1.))
                                    .clamp(0., 1.);
                                    cx.notify();
                                });
                            }
                        });
                        let move_entity = entity.clone();
                        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                            if phase == DispatchPhase::Bubble && event.dragging() {
                                let _ = move_entity.update(cx, |this, cx| {
                                    if !this.dragging {
                                        return;
                                    }
                                    this.value = (f32::from(event.position.x - bounds.left())
                                        / f32::from(bounds.size.width).max(1.))
                                    .clamp(0., 1.);
                                    cx.notify();
                                });
                            }
                        });
                        window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                            if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
                                let _ = entity.update(cx, |this, _| this.dragging = false);
                            }
                        });
                    },
                )
                .absolute()
                .inset_0()
                .size_full(),
            )
    }

    fn content(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .w(px(480.))
                    .p_8()
                    .rounded(px(28.))
                    .bg(rgb(0x19263b))
                    .flex()
                    .flex_col()
                    .gap_6()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(0x91a8c4))
                            .child("POINTER SPACE"),
                    )
                    .child(div().text_size(px(30.)).child("Touch what you see"))
                    .child(div().flex().gap_4().children((0..2).map(|index| {
                        div()
                            .id(("target", index))
                            .flex_1()
                            .h(px(56.))
                            .rounded(px(16.))
                            .bg(rgb(0x314b68))
                            .hover(|s| s.bg(rgb(0x4d839c)))
                            .cursor_pointer()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(format!(
                                "{} · {}",
                                if index == 0 { "Left" } else { "Right" },
                                self.clicks[index]
                            ))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.clicks[index] += 1;
                                cx.notify();
                            }))
                    })))
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .text_sm()
                            .text_color(rgb(0xa3bed2))
                            .child(if self.lens {
                                "MAGNIFICATION"
                            } else {
                                "STRETCH"
                            })
                            .child(if self.lens {
                                format!("{:.2}×", 1. + self.value * 2.)
                            } else {
                                format!("{:.0}%", self.value * 100.)
                            }),
                    )
                    .child(self.slider(cx)),
            )
    }
}

impl Render for InteractionPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let stage = if self.lens {
            EffectStage::lens(LensOptions {
                radius: px(300.),
                magnification: 1. + self.value * 2.,
                ..Default::default()
            })
        } else {
            EffectStage::deformation(DeformationOptions {
                radius: px(330.),
                offset: point(
                    px(110. * self.value * self.direction),
                    px(-24. * self.value),
                ),
                ..Default::default()
            })
        };
        div()
            .size_full()
            .p_8()
            .flex()
            .flex_col()
            .gap_6()
            .bg(rgb(0x0c1422))
            .text_color(rgb(0xe6f3ff))
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_size(px(27.)).child("Interaction mapping"))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(self.control(0, "Lens", self.lens, cx))
                            .child(self.control(1, "Stretch", !self.lens, cx))
                            .child(self.control(
                                2,
                                if self.mapped {
                                    "Mapping on"
                                } else {
                                    "Mapping off"
                                },
                                self.mapped,
                                cx,
                            ))
                            .when(!self.lens, |s| {
                                s.child(self.control(3, "Reverse", false, cx))
                            }),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .rounded(px(28.))
                    .overflow_hidden()
                    .bg(rgb(0x101d2f))
                    .child(
                        subtree_effect_chain(self.content(cx), [stage])
                            .map_interaction(self.mapped),
                    ),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_sm()
                    .text_color(rgb(0x819cb7))
                    .child("Hover or click the buttons. Drag the slider to adjust magnification or stretch."),
            )
    }
}

fn main() {
    application().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1200.), px(800.)),
                    cx,
                ))),
                ..Default::default()
            },
            |_, cx| {
                cx.new(|_| InteractionPreview {
                    lens: true,
                    mapped: true,
                    clicks: [0; 2],
                    value: 0.5,
                    direction: 1.,
                    dragging: false,
                })
            },
        )
        .expect("failed to open interaction mapping example");
    });
}
