use std::{borrow::Cow, fs, path::PathBuf, time::Duration};

use anyhow::Result;
use gpui::{
    Animation, AnimationExt, App, AssetSource, Bounds, Context, Render, SharedString, Window,
    WindowOptions, div, linear_color_stop, multi_linear_gradient, prelude::*, px, rgb, size,
};
use gpui_effects::{gradient_svg, gradient_text};
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

struct GradientMasksExample;

impl Render for GradientMasksExample {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let flowing = multi_linear_gradient(
            90.0,
            [
                linear_color_stop(rgb(0xff375f), 0.0),
                linear_color_stop(rgb(0x7c3aed), 0.33),
                linear_color_stop(rgb(0x00d4aa), 0.66),
                linear_color_stop(rgb(0xffcc00), 1.0),
            ],
        );

        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_10()
            .bg(rgb(0x101218))
            .child(
                gradient_text("One continuous gradient across every glyph", flowing)
                    .text_size(px(54.0))
                    .with_animation(
                        "gradient-text-flow",
                        Animation::new(Duration::from_secs(5)).repeat(),
                        |text, time| text.phase(time),
                    ),
            )
            .child(
                gradient_svg("gradient-mark.svg", flowing)
                    .size(px(240.0))
                    .with_animation(
                        "gradient-svg-flow",
                        Animation::new(Duration::from_secs(5)).repeat(),
                        |svg, time| svg.phase(time),
                    ),
            )
    }
}

fn main() {
    application()
        .with_assets(Assets(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples"),
        ))
        .run(|cx: &mut App| {
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(gpui::WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(1100.0), px(620.0)),
                        cx,
                    ))),
                    ..Default::default()
                },
                |_, cx| cx.new(|_| GradientMasksExample),
            )
            .expect("failed to open gradient mask example");
        });
}
