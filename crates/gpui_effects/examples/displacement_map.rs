use std::{path::PathBuf, sync::Arc, time::Instant};

use gpui::{
    App, Bounds, Context, FontWeight, Image, ImageFormat, ImageSource, ObjectFit, Render,
    RenderImage, Window, WindowBounds, WindowOptions, div, img, point, prelude::*, px, rgb, size,
};
use gpui_effects::{
    DisplacementMapOptions, DisplacementMapPreset, EffectStage, subtree_effect_chain,
};
use gpui_platform::application;

struct MapPreview {
    cover: ImageSource,
    map: ImageSource,
    mask: ImageSource,
    preset: DisplacementMapPreset,
    external: Option<ImageSource>,
    using_external: bool,
    masked: bool,
    paused: bool,
    elapsed: f32,
    last_frame: Instant,
    strength: f32,
    density: f32,
    speed: f32,
}

impl MapPreview {
    fn new() -> Self {
        let external = std::env::args_os()
            .nth(1)
            .map(|path| ImageSource::from(PathBuf::from(path)));
        let mask = std::env::args_os()
            .nth(2)
            .map(|path| ImageSource::from(PathBuf::from(path)))
            .unwrap_or_else(|| {
                let pixels = image::RgbaImage::from_fn(128, 128, |x, y| {
                    let u = (x as f32 + 0.5) / 128. * 2. - 1.;
                    let v = (y as f32 + 0.5) / 128. * 2. - 1.;
                    let t = ((1. - (u * u + v * v).sqrt()) / 0.55).clamp(0., 1.);
                    let coverage = (t * t * (3. - 2. * t) * 255.).round() as u8;
                    image::Rgba([coverage, coverage, coverage, 255])
                });
                Arc::new(RenderImage::new(smallvec::smallvec![image::Frame::new(
                    pixels
                )]))
                .into()
            });
        Self {
            cover: Arc::new(Image::from_bytes(
                ImageFormat::Svg,
                include_bytes!("album-cover.svg").to_vec(),
            ))
            .into(),
            map: external
                .clone()
                .unwrap_or_else(|| DisplacementMapPreset::Water.image()),
            mask,
            preset: DisplacementMapPreset::Water,
            using_external: external.is_some(),
            external,
            masked: false,
            paused: false,
            elapsed: 0.,
            last_frame: Instant::now(),
            strength: 1.,
            density: 1.,
            speed: 1.,
        }
    }

    fn card(&self) -> impl IntoElement {
        div()
            .w_full()
            .flex()
            .flex_col()
            .rounded(px(24.))
            .overflow_hidden()
            .bg(rgb(0x182339))
            .child(
                div()
                    .relative()
                    .w_full()
                    .h(px(210.))
                    .flex_shrink_0()
                    .overflow_hidden()
                    .child(
                        img(self.cover.clone())
                            .absolute()
                            .inset_0()
                            .w_full()
                            .h(px(210.))
                            .max_h(px(210.))
                            .min_w_0()
                            .min_h_0()
                            .object_fit(ObjectFit::Cover),
                    ),
            )
            .child(
                div()
                    .p_6()
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(0x8eabc5))
                            .child("SOUND IN MOTION / 01"),
                    )
                    .child(
                        div()
                            .text_size(px(42.))
                            .font_weight(FontWeight::BOLD)
                            .child("Refraction"),
                    )
                    .child(
                        div()
                            .text_size(px(21.))
                            .text_color(rgb(0xbcd0e1))
                            .child("水面之下 · Beneath the surface"),
                    )
                    .child(
                        div()
                            .mt_4()
                            .flex()
                            .justify_between()
                            .text_sm()
                            .text_color(rgb(0x7f9bb7))
                            .child("Quiet waves. Moving light.")
                            .child("04:28"),
                    ),
            )
    }

    fn button(
        &self,
        index: usize,
        label: &'static str,
        active: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id(("mode", index))
            .px_4()
            .py_2()
            .rounded_full()
            .cursor_pointer()
            .text_sm()
            .bg(rgb(if active { 0x315678 } else { 0x1b2a40 }))
            .hover(|s| s.bg(rgb(0x3b6081)))
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                match index {
                    0 | 1 => {
                        this.preset = if index == 0 {
                            DisplacementMapPreset::Water
                        } else {
                            DisplacementMapPreset::Heat
                        };
                        this.map = this.preset.image();
                        this.using_external = false;
                        this.elapsed = 0.;
                    }
                    2 => this.masked = !this.masked,
                    3 => this.paused = !this.paused,
                    _ => {
                        if let Some(external) = &this.external {
                            this.map = external.clone();
                            this.using_external = true;
                        }
                    }
                }
                this.last_frame = Instant::now();
                cx.notify();
            }))
    }

    fn control(
        &self,
        index: usize,
        label: &'static str,
        value: f32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .flex_1()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().text_xs().text_color(rgb(0x879db9)).child(label))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .child(format!("{value:.2}×"))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .children([-1., 1.].into_iter().enumerate().map(|(button, delta)| {
                                div()
                                    .id(("adjust", index * 2 + button))
                                    .size(px(30.))
                                    .rounded(px(8.))
                                    .bg(rgb(0x26364e))
                                    .hover(|s| s.bg(rgb(0x3a506c)))
                                    .cursor_pointer()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(if delta < 0. { "−" } else { "+" })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        match index {
                                            0 => {
                                                this.strength =
                                                    (this.strength + delta * 0.25).clamp(0., 3.)
                                            }
                                            1 => {
                                                this.density =
                                                    (this.density + delta * 0.25).clamp(0.25, 4.)
                                            }
                                            _ => {
                                                this.speed =
                                                    (this.speed + delta * 0.25).clamp(0., 3.)
                                            }
                                        }
                                        this.last_frame = Instant::now();
                                        cx.notify();
                                    }))
                            })),
                    ),
            )
    }
}

impl Render for MapPreview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
        if !self.paused
            && self.strength > 0.
            && self.speed > 0.
            && window.supports_subtree_effects()
        {
            self.elapsed +=
                now.duration_since(self.last_frame).as_secs_f32().min(0.05) * self.speed;
            window.request_animation_frame();
        }
        self.last_frame = now;
        let base = self.preset.options();
        let options = DisplacementMapOptions {
            amplitude: base.amplitude * self.strength,
            scale: point(self.density, self.density),
            ..base
        };
        let stage = if self.masked {
            EffectStage::masked_displacement_map(self.map.clone(), self.mask.clone(), options)
        } else {
            EffectStage::displacement_map(self.map.clone(), options)
        };
        div()
            .size_full()
            .p_8()
            .flex()
            .flex_col()
            .gap_6()
            .bg(rgb(0x0b1220))
            .text_color(rgb(0xe9f2ff))
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(div().text_size(px(28.)).child("Surface currents"))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(0x8299b6))
                                    .child("Texture-driven displacement"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(self.button(
                                0,
                                "Water",
                                !self.using_external && self.preset == DisplacementMapPreset::Water,
                                cx,
                            ))
                            .child(self.button(
                                1,
                                "Heat",
                                !self.using_external && self.preset == DisplacementMapPreset::Heat,
                                cx,
                            ))
                            .when(self.external.is_some(), |s| {
                                s.child(self.button(4, "External", self.using_external, cx))
                            })
                            .child(self.button(
                                3,
                                if self.paused { "Play" } else { "Pause" },
                                self.paused,
                                cx,
                            )),
                    ),
            )
            .child(
                div()
                    .id("preview")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .min_h_full()
                            .px_6()
                            .py_6()
                            .flex()
                            .items_center()
                            .gap_8()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap_5()
                                    .child(
                                        div().text_xs().text_color(rgb(0x879db9)).child("SOURCE"),
                                    )
                                    .child(self.card()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap_5()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(0x8fd8e2))
                                            .child("DISPLACED"),
                                    )
                                    .child(
                                        subtree_effect_chain(self.card(), [stage])
                                            .time(self.elapsed),
                                    ),
                            ),
                    ),
            )
            .child(
                div()
                    .p_5()
                    .flex_shrink_0()
                    .rounded(px(20.))
                    .bg(rgb(0x131f31))
                    .flex()
                    .items_center()
                    .gap_8()
                    .child(img(self.map.clone()).size(px(72.)).rounded(px(12.)))
                    .child(self.control(0, "STRENGTH", self.strength, cx))
                    .child(self.control(1, "MAP DENSITY", self.density, cx))
                    .child(self.control(2, "SPEED", self.speed, cx))
                    .child(self.button(2, "Local mask", self.masked, cx)),
            )
            .when(!window.supports_subtree_effects(), |s| {
                s.child("Subtree effects are unavailable on this renderer.")
            })
    }
}

fn main() {
    application().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1280.), px(850.)),
                    cx,
                ))),
                ..Default::default()
            },
            |_, cx| cx.new(|_| MapPreview::new()),
        )
        .expect("failed to open displacement map example");
    });
}
