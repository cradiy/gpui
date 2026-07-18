use std::{borrow::Cow, fs, path::PathBuf, time::Duration};

use anyhow::Result;
use gpui::{
    Animation, AnimationExt, App, AssetSource, Bounds, Context, EffectShader, Render, SharedString,
    Transformation, Window, WindowBounds, WindowOptions, div, prelude::*, px, rgb, size,
};
use gpui_effects::{effect_text, spectrum_svg, spectrum_text};
use gpui_platform::application;

struct Assets(PathBuf);

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        fs::read(self.0.join(path))
            .map(|bytes| Some(Cow::Owned(bytes)))
            .map_err(Into::into)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        fs::read_dir(self.0.join(path))?
            .map(|entry| {
                entry.map_err(Into::into).and_then(|entry| {
                    entry
                        .file_name()
                        .into_string()
                        .map(SharedString::from)
                        .map_err(|_| anyhow::anyhow!("asset name is not UTF-8"))
                })
            })
            .collect()
    }
}

struct MaskedEffectsExample;

impl Render for MaskedEffectsExample {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let colors = [rgb(0xff375f), rgb(0x7c3aed), rgb(0x00d4aa), rgb(0xffcc00)];
        let custom_shader = EffectShader::wgsl_mask(CUSTOM_MASK_WGSL);

        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_10()
            .bg(rgb(0x101218))
            .child(
                spectrum_text("WGSL flowing through a shared text mask", colors)
                    .text_size(px(52.0))
                    .with_animation(
                        "spectrum-text",
                        Animation::new(Duration::from_secs(5)).repeat(),
                        |text, time| text.time(time),
                    ),
            )
            .child(
                effect_text("Custom shader: no renderer changes", custom_shader)
                    .text_size(px(38.0))
                    .uniform(0, [0.15, 0.72, 1.0, 1.0])
                    .uniform(1, [1.0, 0.2, 0.56, 1.0])
                    .with_animation(
                        "custom-mask-text",
                        Animation::new(Duration::from_secs(4)).repeat(),
                        |text, time| text.time(time),
                    ),
            )
            .child(
                spectrum_svg("gradient-mark.svg", colors)
                    .size(px(220.0))
                    .with_transformation(Transformation::scale(size(0.92, 0.92)))
                    .with_animation(
                        "spectrum-svg",
                        Animation::new(Duration::from_secs(5)).repeat(),
                        |svg, time| svg.time(time),
                    ),
            )
    }
}

const CUSTOM_MASK_WGSL: &str = r#"
fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> {
    let tau = 6.28318530718;
    let phase = input.time * tau;
    let diagonal = input.uv.x * 1.4 + input.uv.y * 0.6;
    let wave = 0.5 + 0.5 * sin(diagonal * tau * 1.5 - phase);
    let color = mix(params.slots[0], params.slots[1], wave);
    let highlight = pow(0.5 + 0.5 * sin(diagonal * tau * 3.0 - phase * 2.0), 8.0);
    return vec4<f32>(color.rgb + vec3<f32>(highlight * 0.22), color.a);
}
"#;

fn main() {
    application()
        .with_assets(Assets(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples"),
        ))
        .run(|cx: &mut App| {
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(1180.0), px(720.0)),
                        cx,
                    ))),
                    ..Default::default()
                },
                |_, cx| cx.new(|_| MaskedEffectsExample),
            )
            .expect("failed to open masked effects example");
        });
}
