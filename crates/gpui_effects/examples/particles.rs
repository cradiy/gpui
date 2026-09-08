use std::{cell::Cell, rc::Rc, time::Duration};

use gpui::{
    App, Bounds, Context, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Point, Render,
    Window, WindowBounds, WindowOptions, canvas, div, prelude::*, px, rgb, size,
};
use gpui_effects::{BloomOptions, EffectStage, ParticleSpawn, Particles, subtree_effect_chain};
use gpui_platform::application;

const COLORS: [u32; 3] = [0x9deaff, 0xc4afff, 0xffcfa0];

struct ParticlePreview {
    particles: Particles,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    previous: Option<Point<Pixels>>,
    remainder: f32,
    color: usize,
    force: i32,
    streaks: bool,
    bloom: bool,
}

impl ParticlePreview {
    fn new() -> Self {
        Self {
            particles: Particles::default(),
            bounds: Rc::new(Cell::new(Bounds::default())),
            previous: None,
            remainder: 0.,
            color: 0,
            force: 0,
            streaks: true,
            bloom: true,
        }
    }

    fn motion(&mut self, position: Point<Pixels>) {
        let local = position - self.bounds.get().origin;
        let mut physics = self.particles.physics();
        physics.attractor = local;
        physics.strength = px(self.force as f32 * 750.);
        self.particles.set_physics(physics);
        if self.particles.is_paused() {
            self.previous = None;
            return;
        }
        if let Some(previous) = self.previous {
            let delta = local - previous;
            let distance = f32::from(delta.x).hypot(f32::from(delta.y));
            let amount = distance * 0.8 + self.remainder;
            let count = (amount.floor() as u32).min(128);
            self.remainder = amount.fract();
            if count > 0 {
                self.particles.emit(ParticleSpawn {
                    from: previous,
                    to: local,
                    count,
                    color: rgb(COLORS[self.color]),
                    stretch: if self.streaks { 0.04 } else { 0. },
                    ..Default::default()
                });
            }
        }
        self.previous = Some(local);
    }

    fn burst(&mut self, position: Point<Pixels>) {
        let local = position - self.bounds.get().origin;
        for color in COLORS {
            self.particles.emit(ParticleSpawn {
                from: local,
                to: local,
                count: 90,
                speed: px(70.)..px(320.),
                lifetime: Duration::from_millis(900)..Duration::from_millis(2200),
                color: rgb(color),
                stretch: if self.streaks { 0.04 } else { 0. },
                ..Default::default()
            });
        }
    }
}

impl Render for ParticlePreview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bounds = self.bounds.clone();
        let frame = self.particles.frame();
        let source = canvas(
            move |region, _, _| bounds.set(region),
            move |region, _, window, _| window.paint_particles(region, frame),
        )
        .size_full();
        let surface = subtree_effect_chain(
            source,
            [EffectStage::bloom(BloomOptions {
                radius: px(18.),
                intensity: 1.3,
                threshold: 0.3,
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
                            .child(div().text_size(px(28.)).child("Stardust"))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(0x8492b0))
                                    .child("Move to scatter light. Click to burst."),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_3()
                            .child(
                                div()
                                    .id("pause")
                                    .px_4()
                                    .py_2()
                                    .rounded_full()
                                    .bg(rgb(0x293854))
                                    .cursor_pointer()
                                    .child(if self.particles.is_paused() {
                                        "Resume"
                                    } else {
                                        "Pause"
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.particles.set_paused(!this.particles.is_paused());
                                        this.previous = None;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                div()
                                    .id("clear")
                                    .px_4()
                                    .py_2()
                                    .rounded_full()
                                    .bg(rgb(0x1b2336))
                                    .cursor_pointer()
                                    .child("Clear")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.particles.clear();
                                        this.previous = None;
                                        this.remainder = 0.;
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
            .child(
                div()
                    .id("particle-surface")
                    .relative()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .rounded(px(24.))
                    .overflow_hidden()
                    .bg(rgb(0x101727))
                    .cursor_crosshair()
                    .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                        this.motion(event.position);
                        cx.notify();
                    }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, _, cx| {
                            this.burst(event.position);
                            cx.notify();
                        }),
                    )
                    .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                        if !*hovered {
                            this.previous = None;
                            let mut physics = this.particles.physics();
                            physics.strength = px(0.);
                            this.particles.set_physics(physics);
                            cx.notify();
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
                    .child(
                        div()
                            .flex()
                            .gap_3()
                            .children(COLORS.into_iter().enumerate().map(|(index, color)| {
                                div()
                                    .id(("color", index))
                                    .size(px(28.))
                                    .rounded_full()
                                    .bg(rgb(color))
                                    .cursor_pointer()
                                    .opacity(if self.color == index { 1. } else { 0.45 })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.color = index;
                                        cx.notify();
                                    }))
                            })),
                    )
                    .child(
                        div().flex().gap_2().children(
                            [("Free", 0), ("Attract", 1), ("Repel", -1)]
                                .into_iter()
                                .enumerate()
                                .map(|(index, (label, force))| {
                                    div()
                                        .id(("force", index))
                                        .px_3()
                                        .py_2()
                                        .rounded_full()
                                        .bg(rgb(if self.force == force {
                                            0x344665
                                        } else {
                                            0x1b2336
                                        }))
                                        .cursor_pointer()
                                        .child(label)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.force = force;
                                            cx.notify();
                                        }))
                                }),
                        ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                div()
                                    .id("shape")
                                    .px_3()
                                    .py_2()
                                    .rounded_full()
                                    .bg(rgb(0x1b2336))
                                    .cursor_pointer()
                                    .child(if self.streaks {
                                        "Light streaks"
                                    } else {
                                        "Light points"
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.streaks = !this.streaks;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                div()
                                    .id("bloom")
                                    .px_3()
                                    .py_2()
                                    .rounded_full()
                                    .bg(rgb(0x1b2336))
                                    .cursor_pointer()
                                    .child(if self.bloom {
                                        "Bloom · On"
                                    } else {
                                        "Bloom · Off"
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.bloom = !this.bloom;
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
            .when(!window.supports_gpu_particles(), |root| {
                root.child("GPU particles are unavailable on this renderer.")
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
            |_, cx| cx.new(|_| ParticlePreview::new()),
        )
        .expect("failed to open particles example");
    });
}
