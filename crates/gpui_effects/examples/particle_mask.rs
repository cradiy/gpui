use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::{
    App, Bounds, Context, FontWeight, Image, ImageFormat, ImageSource, Render, Window,
    WindowBounds, WindowOptions, div, img, point, prelude::*, px, rgb, size,
};
use gpui_effects::{ParticleMask, ParticleSpawn, Particles, subtree_particles};
use gpui_platform::application;

const ART: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="320" height="240" viewBox="0 0 320 240">
<defs><linearGradient id="color" x1="0" y1="1" x2="1" y2="0"><stop stop-color="#73e6ee"/><stop offset=".5" stop-color="#a5b7ff"/><stop offset="1" stop-color="#f1b3ee"/></linearGradient></defs>
<g fill="none" stroke="url(#color)" stroke-width="7" stroke-linecap="round">
<ellipse cx="160" cy="120" rx="112" ry="45" transform="rotate(-35 160 120)"/>
<ellipse cx="160" cy="120" rx="112" ry="45" transform="rotate(35 160 120)"/>
<circle cx="160" cy="120" r="54"/></g>
<circle cx="160" cy="120" r="16" fill="#c3e4ff"/></svg>"##;

struct MaskPreview {
    particles: Particles,
    last_frame: Instant,
    remainder: f32,
    density: f32,
    speed: f32,
    lifetime: f32,
    mask: ParticleMask,
    artwork: bool,
    image: ImageSource,
}

impl MaskPreview {
    fn new() -> Self {
        let mut particles = Particles::new(4096);
        particles.set_physics(gpui_effects::ParticlePhysics {
            acceleration: point(px(3.), px(-9.)),
            drag: 0.35,
            ..Default::default()
        });
        Self {
            particles,
            last_frame: Instant::now(),
            remainder: 0.,
            density: 100.,
            speed: 30.,
            lifetime: 2.4,
            mask: ParticleMask::default(),
            artwork: false,
            image: Arc::new(Image::from_bytes(ImageFormat::Svg, ART.to_vec())).into(),
        }
    }

    fn control(
        &self,
        index: usize,
        label: &'static str,
        value: String,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        div()
            .flex_1()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().text_xs().text_color(rgb(0x8595b3)).child(label))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .child(value)
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .children([-1., 1.].into_iter().enumerate().map(|(button, delta)| {
                                div()
                                    .id(("adjust", index * 2 + button))
                                    .size(px(30.))
                                    .rounded(px(9.))
                                    .bg(rgb(0x253149))
                                    .hover(|s| s.bg(rgb(0x35445f)))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .child(if delta < 0. { "−" } else { "+" })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        match index {
                                            0 => {
                                                this.density =
                                                    (this.density + delta * 25.).clamp(0., 500.)
                                            }
                                            1 => {
                                                this.speed =
                                                    (this.speed + delta * 5.).clamp(5., 100.)
                                            }
                                            _ => {
                                                this.lifetime =
                                                    (this.lifetime + delta * 0.4).clamp(0.4, 5.2)
                                            }
                                        }
                                        cx.notify();
                                    }))
                            })),
                    ),
            )
    }

    fn button(
        &self,
        index: usize,
        label: &'static str,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id(("mode", index))
            .px_4()
            .py_2()
            .rounded_full()
            .bg(rgb(0x24314a))
            .hover(|s| s.bg(rgb(0x34445f)))
            .text_sm()
            .cursor_pointer()
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                match index {
                    0 => {
                        this.particles.set_paused(!this.particles.is_paused());
                        this.last_frame = Instant::now();
                    }
                    1 => {
                        this.artwork = !this.artwork;
                        this.particles.clear();
                        this.remainder = 0.;
                    }
                    2 => {
                        this.mask.edge_width = if this.mask.edge_width == px(0.) {
                            px(2.)
                        } else {
                            px(0.)
                        }
                    }
                    _ => this.mask.inherit_color = !this.mask.inherit_color,
                }
                cx.notify();
            }))
    }
}

impl Render for MaskPreview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_frame).as_secs_f32().min(0.05);
        self.last_frame = now;
        if !self.particles.is_paused() && window.supports_gpu_particles() {
            let amount = self.density * elapsed + self.remainder;
            let count = amount.floor() as u32;
            self.remainder = amount.fract();
            if count > 0 {
                self.particles.emit(ParticleSpawn {
                    count,
                    velocity: point(px(0.), px(-self.speed)),
                    speed: px(self.speed * 0.15)..px(self.speed * 0.6),
                    lifetime: Duration::from_secs_f32(self.lifetime * 0.6)
                        ..Duration::from_secs_f32(self.lifetime),
                    radius: px(0.6)..px(1.25),
                    color: rgb(0x9feaff),
                    stretch: 0.025,
                    ..Default::default()
                });
            }
            if self.density > 0. {
                window.request_animation_frame();
            }
        }
        let content = div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_5()
            .when(self.artwork, |s| {
                s.child(img(self.image.clone()).w(px(320.)).h(px(240.)))
            })
            .when(!self.artwork, |s| {
                s.child(
                    div()
                        .text_size(px(96.))
                        .line_height(px(120.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(0x9fe7f2))
                        .child("Luminous"),
                )
                .child(
                    div()
                        .text_size(px(38.))
                        .text_color(rgb(0xc6b6f1))
                        .child("浮光 · 星尘"),
                )
            });
        let surface = subtree_particles(content, &mut self.particles, self.mask);
        div()
            .size_full()
            .p_8()
            .bg(rgb(0x0a101c))
            .text_color(rgb(0xe7efff))
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
                            .child(div().text_size(px(28.)).child("Light from form"))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(0x8192af))
                                    .child("Particles shaped by text and artwork."),
                            ),
                    )
                    .child(self.button(
                        0,
                        if self.particles.is_paused() {
                            "Resume"
                        } else {
                            "Pause"
                        },
                        cx,
                    )),
            )
            .child(
                div()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .rounded(px(28.))
                    .overflow_hidden()
                    .bg(rgb(0x101a2b))
                    .child(surface),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(self.button(1, if self.artwork { "Artwork" } else { "Text" }, cx))
                    .child(self.button(
                        2,
                        if self.mask.edge_width == px(0.) {
                            "Fill"
                        } else {
                            "Edges"
                        },
                        cx,
                    ))
                    .child(self.button(
                        3,
                        if self.mask.inherit_color {
                            "Source color"
                        } else {
                            "Ice blue"
                        },
                        cx,
                    )),
            )
            .child(
                div()
                    .p_5()
                    .rounded(px(20.))
                    .bg(rgb(0x141f31))
                    .flex()
                    .gap_8()
                    .child(self.control(0, "DENSITY", format!("{:.0} / s", self.density), cx))
                    .child(self.control(1, "SPEED", format!("{:.0} px / s", self.speed), cx))
                    .child(self.control(2, "LIFETIME", format!("{:.1} s", self.lifetime), cx)),
            )
            .when(!window.supports_gpu_particles(), |s| {
                s.child("GPU particles are unavailable on this renderer.")
            })
    }
}

fn main() {
    application().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1100.), px(820.)),
                    cx,
                ))),
                ..Default::default()
            },
            |_, cx| cx.new(|_| MaskPreview::new()),
        )
        .expect("failed to open particle mask example");
    });
}
