use std::{path::PathBuf, sync::Arc, time::Instant};

use gpui::{
    App, Bounds, Context, Image, ImageFormat, ImageSource, ObjectFit, Render, Window, WindowBounds,
    WindowOptions, div, img, prelude::*, px, rgba, size,
};
use gpui_effects::{ColorFlowOptions, color_flow};
use gpui_platform::application;

#[path = "color_flow/palette.rs"]
mod palette;

struct PreviewSettings {
    speed: f32,
    effect: ColorFlowOptions,
    brightness_step: f32,
    brightness_min: f32,
    brightness_max: f32,
    cohesion_step: f32,
    tone_step: f32,
    use_extracted_palette: bool,
    sample_grid: usize,
    cluster_iterations: usize,
}

impl Default for PreviewSettings {
    fn default() -> Self {
        Self {
            speed: 1.0,
            effect: ColorFlowOptions::default(),
            brightness_step: 0.05,
            brightness_min: 0.0,
            brightness_max: 2.0,
            cohesion_step: 0.1,
            tone_step: 0.05,
            use_extracted_palette: false,
            sample_grid: 32,
            cluster_iterations: 8,
        }
    }
}

struct ColorFlowPreview {
    cover: ImageSource,
    palette: Option<palette::Palette>,
    settings: PreviewSettings,
    last_frame: Instant,
    elapsed: f32,
    paused: bool,
}

impl ColorFlowPreview {
    fn new(cover: ImageSource) -> Self {
        Self {
            cover,
            palette: None,
            settings: PreviewSettings::default(),
            last_frame: Instant::now(),
            elapsed: 0.0,
            paused: false,
        }
    }
}

impl Render for ColorFlowPreview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
        if !self.paused {
            self.elapsed += now.duration_since(self.last_frame).as_secs_f32() * self.settings.speed;
            window.request_animation_frame();
        }
        self.last_frame = now;
        if self.palette.is_none()
            && let Some(Ok(image)) = self.cover.use_data(None, window, cx)
            && let Some(bytes) = image.as_bytes(0)
        {
            let size = image.size(0);
            self.palette = Some(palette::extract(
                bytes,
                size.width.0 as usize,
                size.height.0 as usize,
                self.settings.sample_grid,
                self.settings.cluster_iterations,
            ));
        }
        let glow = color_flow(self.cover.clone())
            .options(self.settings.effect)
            .palette(if self.settings.use_extracted_palette {
                self.palette
            } else {
                None
            })
            .time(self.elapsed);
        let control = |id: &'static str, label: String| {
            div()
                .id(id)
                .px_3()
                .py_2()
                .rounded_lg()
                .bg(rgba(0xffffff18))
                .text_color(gpui::rgb(0xffffff))
                .cursor_pointer()
                .hover(|style| style.bg(rgba(0xffffff30)))
                .child(label)
        };

        div()
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(gpui::rgb(0x171922))
            .child(glow.absolute().inset_0())
            .child(
                img(self.cover.clone())
                    .id("album-cover")
                    .absolute()
                    .left(px(48.))
                    .bottom(px(48.))
                    .size(px(280.))
                    .object_fit(ObjectFit::Cover)
                    .rounded(px(28.))
                    .shadow(vec![
                        gpui::BoxShadow::new(px(0.), px(24.), rgba(0x00000080).into())
                            .blur_radius(px(64.))
                            .spread_radius(px(-16.)),
                    ]),
            )
            .child(
                div()
                    .absolute()
                    .right(px(32.))
                    .bottom(px(32.))
                    .p_4()
                    .rounded_xl()
                    .bg(rgba(0x101014aa))
                    .text_color(gpui::rgb(0xffffff))
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(div().text_sm().child("EXTRACTED PALETTE / FLOW"))
                    .child(div().flex().gap_2().children(
                        self.palette.unwrap_or_default().into_iter().map(|color| {
                            div()
                                .w(px(48.))
                                .h(px(24.))
                                .rounded_md()
                                .bg(color.color)
                                .opacity(if color.weight > 0.0 { 1.0 } else { 0.2 })
                        }),
                    ))
                    .child(
                        control(
                            "palette-mode",
                            if self.settings.use_extracted_palette {
                                "Source: extracted palette"
                            } else {
                                "Source: image sampling"
                            }
                            .into(),
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.settings.use_extracted_palette =
                                !this.settings.use_extracted_palette;
                            cx.notify();
                        })),
                    )
                    .children(
                        [
                            ToneControl::Shadows,
                            ToneControl::Highlights,
                            ToneControl::Neutrals,
                        ]
                        .into_iter()
                        .map(|tone| {
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(div().flex_1().text_sm().child(format!(
                                    "{} {:.0}%",
                                    tone.label(),
                                    *tone.value(&mut self.settings.effect) * 100.0
                                )))
                                .child(control(tone.decrease_id(), "−".into()).on_click(
                                    cx.listener(move |this, _, _, cx| {
                                        let value = tone.value(&mut this.settings.effect);
                                        *value = (*value - this.settings.tone_step).clamp(0.0, 1.0);
                                        cx.notify();
                                    }),
                                ))
                                .child(control(tone.increase_id(), "+".into()).on_click(
                                    cx.listener(move |this, _, _, cx| {
                                        let value = tone.value(&mut this.settings.effect);
                                        *value = (*value + this.settings.tone_step).clamp(0.0, 1.0);
                                        cx.notify();
                                    }),
                                ))
                        }),
                    )
                    .child(div().text_sm().child(format!(
                        "Speed {:.2}× · Diffusion {:.2}",
                        self.settings.speed, self.settings.effect.diffusion
                    )))
                    .child(div().text_sm().child(format!(
                        "Cohesion {:.0}%",
                        self.settings.effect.cohesion * 100.0
                    )))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(control("looser", "Looser".into()).on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.settings.effect.cohesion = (this.settings.effect.cohesion
                                        - this.settings.cohesion_step)
                                        .clamp(0.0, 1.0);
                                    cx.notify();
                                },
                            )))
                            .child(control("tighter", "Tighter".into()).on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.settings.effect.cohesion = (this.settings.effect.cohesion
                                        + this.settings.cohesion_step)
                                        .clamp(0.0, 1.0);
                                    cx.notify();
                                },
                            ))),
                    )
                    .child(div().text_sm().child(format!(
                        "Background brightness {:.0}%",
                        self.settings.effect.brightness * 100.0
                    )))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(control("dimmer", "Dimmer".into()).on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.settings.effect.brightness =
                                        (this.settings.effect.brightness
                                            - this.settings.brightness_step)
                                            .clamp(
                                                this.settings.brightness_min,
                                                this.settings.brightness_max,
                                            );
                                    cx.notify();
                                },
                            )))
                            .child(control("brighter", "Brighter".into()).on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.settings.effect.brightness =
                                        (this.settings.effect.brightness
                                            + this.settings.brightness_step)
                                            .clamp(
                                                this.settings.brightness_min,
                                                this.settings.brightness_max,
                                            );
                                    cx.notify();
                                },
                            ))),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(control("slower", "Slower".into()).on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.settings.speed = (this.settings.speed - 0.25).max(0.25);
                                    cx.notify();
                                },
                            )))
                            .child(control("faster", "Faster".into()).on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.settings.speed = (this.settings.speed + 0.25).min(3.0);
                                    cx.notify();
                                },
                            )))
                            .child(
                                control(
                                    "pause",
                                    if self.paused { "Resume" } else { "Pause" }.into(),
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.paused = !this.paused;
                                        this.last_frame = Instant::now();
                                        cx.notify();
                                    },
                                )),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                control("defined", "Less diffuse".into()).on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.settings.effect.diffusion =
                                            (this.settings.effect.diffusion - 0.03).max(0.06);
                                        cx.notify();
                                    },
                                )),
                            )
                            .child(control("diffuse", "More diffuse".into()).on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.settings.effect.diffusion =
                                        (this.settings.effect.diffusion + 0.03).min(0.3);
                                    cx.notify();
                                }),
                            )),
                    ),
            )
    }
}

#[derive(Clone, Copy)]
enum ToneControl {
    Shadows,
    Highlights,
    Neutrals,
}

impl ToneControl {
    fn value(self, options: &mut ColorFlowOptions) -> &mut f32 {
        match self {
            Self::Shadows => &mut options.shadow_level,
            Self::Highlights => &mut options.highlight_level,
            Self::Neutrals => &mut options.neutral_weight,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Shadows => "Shadow level",
            Self::Highlights => "Highlight level",
            Self::Neutrals => "Neutral weight",
        }
    }

    fn decrease_id(self) -> &'static str {
        match self {
            Self::Shadows => "shadows-less",
            Self::Highlights => "highlights-less",
            Self::Neutrals => "neutrals-less",
        }
    }

    fn increase_id(self) -> &'static str {
        match self {
            Self::Shadows => "shadows-more",
            Self::Highlights => "highlights-more",
            Self::Neutrals => "neutrals-more",
        }
    }
}

fn cover_source() -> ImageSource {
    std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .map(ImageSource::from)
        .unwrap_or_else(|| {
            Arc::new(Image::from_bytes(
                ImageFormat::Svg,
                include_bytes!("album-cover.svg").to_vec(),
            ))
            .into()
        })
}

fn main() {
    application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|_| ColorFlowPreview::new(cover_source())),
        )
        .expect("failed to open color flow preview");
    });
}
