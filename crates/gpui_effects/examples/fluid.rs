use gpui::{
    App, Bounds, Context, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Point, Render,
    Window, WindowBounds, WindowOptions, canvas, div, prelude::*, px, rgb, size,
};
use gpui_effects::{BloomOptions, EffectStage, Fluid, FluidSplat, subtree_effect_chain};
use gpui_platform::application;
use std::{cell::Cell, rc::Rc, time::Instant};

const COLORS: [u32; 3] = [0x48dfff, 0xb982ff, 0xff8b71];
struct FluidPreview {
    fluid: Fluid,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    previous: Option<(Point<Pixels>, Instant)>,
    color: usize,
    stir: bool,
    bloom: bool,
}
impl FluidPreview {
    fn new() -> Self {
        Self {
            fluid: Fluid::default(),
            bounds: Rc::new(Cell::new(Bounds::default())),
            previous: None,
            color: 0,
            stir: false,
            bloom: true,
        }
    }
    fn inject(&mut self, position: Point<Pixels>) {
        let now = Instant::now();
        let local = position - self.bounds.get().origin;
        let (from, elapsed) = self.previous.map_or((local, 1. / 60.), |(p, t)| {
            (p, now.duration_since(t).as_secs_f32().max(1. / 240.))
        });
        let delta = local - from;
        let distance = f32::from(delta.x).hypot(f32::from(delta.y));
        let gain = (1. / elapsed).min(1400. / distance.max(1.));
        self.fluid.splat(FluidSplat {
            from,
            to: local,
            velocity: delta * gain,
            radius: px(32.),
            amount: if self.stir {
                0.
            } else if self.previous.is_none() {
                1.8
            } else {
                (distance / 32.).clamp(0.08, 0.9)
            },
            color: rgb(COLORS[self.color]),
        });
        self.previous = Some((local, now));
    }
}

fn button(id: &'static str, text: impl Into<gpui::SharedString>) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .px_4()
        .py_2()
        .rounded_full()
        .bg(rgb(0x202d43))
        .cursor_pointer()
        .hover(|style| style.bg(rgb(0x2c3c56)))
        .child(text.into())
}

impl Render for FluidPreview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bounds = self.bounds.clone();
        let frame = self.fluid.frame();
        let source = canvas(
            move |region, _, _| bounds.set(region),
            move |region, _, window, _| window.paint_fluid(region, frame),
        )
        .size_full();
        let surface = subtree_effect_chain(
            source,
            [EffectStage::bloom(BloomOptions {
                threshold: 0.35,
                intensity: 0.7,
                radius: px(20.),
                ..Default::default()
            })
            .enabled(self.bloom)],
        )
        .capture_padding(px(24.));
        div()
            .size_full()
            .p_8()
            .bg(rgb(0x090e19))
            .text_color(rgb(0xececff))
            .flex()
            .flex_col()
            .gap_6()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(div().text_size(px(28.)).child("Fluid ink"))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(0x8492b0))
                                    .child("Drag to pour color. Release to let it flow."),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_3()
                            .child(
                                button(
                                    "pause",
                                    if self.fluid.is_paused() {
                                        "Resume"
                                    } else {
                                        "Pause"
                                    },
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.fluid.set_paused(!this.fluid.is_paused());
                                        this.previous = None;
                                        cx.notify();
                                    },
                                )),
                            )
                            .child(button("clear", "Clear").on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.fluid.clear();
                                    this.previous = None;
                                    cx.notify();
                                },
                            ))),
                    ),
            )
            .child(
                div()
                    .id("fluid-surface")
                    .relative()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .rounded(px(24.))
                    .overflow_hidden()
                    .bg(rgb(0x101727))
                    .cursor_crosshair()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, _, cx| {
                            this.previous = None;
                            this.inject(event.position);
                            cx.notify();
                        }),
                    )
                    .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                        if event.pressed_button == Some(MouseButton::Left)
                            && !this.fluid.is_paused()
                        {
                            this.inject(event.position);
                            cx.notify();
                        } else {
                            this.previous = None;
                        }
                    }))
                    .on_hover(cx.listener(|this, hovered: &bool, _, _| {
                        if !*hovered {
                            this.previous = None;
                        }
                    }))
                    .child(surface),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .child(div().flex().items_center().gap_3().children(
                        COLORS.into_iter().enumerate().map(|(index, color)| {
                            div()
                                .id(("color", index))
                                .size(px(30.))
                                .rounded_full()
                                .bg(rgb(color))
                                .cursor_pointer()
                                .border_2()
                                .border_color(if self.color == index {
                                    rgb(0xffffff)
                                } else {
                                    rgb(0x172139)
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.color = index;
                                    cx.notify();
                                }))
                        }),
                    ))
                    .child(
                        div()
                            .flex()
                            .gap_3()
                            .child(
                                button("mode", if self.stir { "Stir" } else { "Ink" }).on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.stir = !this.stir;
                                        cx.notify();
                                    }),
                                ),
                            )
                            .child(
                                button(
                                    "vorticity",
                                    format!("Swirl · {:.0}", self.fluid.options().vorticity),
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        let mut options = this.fluid.options();
                                        options.vorticity =
                                            if options.vorticity < 15. { 22. } else { 12. };
                                        this.fluid.set_options(options);
                                        cx.notify();
                                    },
                                )),
                            )
                            .child(
                                button(
                                    "resolution",
                                    format!("Grid · {}", self.fluid.options().resolution),
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        let mut options = this.fluid.options();
                                        options.resolution =
                                            if options.resolution == 256 { 128 } else { 256 };
                                        this.fluid.set_options(options);
                                        this.previous = None;
                                        cx.notify();
                                    },
                                )),
                            )
                            .child(
                                button(
                                    "bloom",
                                    if self.bloom {
                                        "Bloom · On"
                                    } else {
                                        "Bloom · Off"
                                    },
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.bloom = !this.bloom;
                                        cx.notify();
                                    },
                                )),
                            ),
                    ),
            )
            .when(!window.supports_gpu_fluid(), |root| {
                root.child("GPU fluid is unavailable on this renderer.")
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
            |_, cx| cx.new(|_| FluidPreview::new()),
        )
        .expect("failed to open fluid example");
    });
}
