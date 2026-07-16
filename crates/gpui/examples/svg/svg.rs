#![cfg_attr(target_family = "wasm", no_main)]

use std::fs;
use std::path::PathBuf;

use anyhow::Result;
use gpui::{
    App, AssetSource, Bounds, Context, SharedString, Transformation, Window, WindowBounds,
    WindowOptions, color_svg, div, prelude::*, px, radians, rgb, size, svg,
};
use gpui_platform::application;

struct Assets {
    base: PathBuf,
}

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<std::borrow::Cow<'static, [u8]>>> {
        fs::read(self.base.join(path))
            .map(|data| Some(std::borrow::Cow::Owned(data)))
            .map_err(|err| err.into())
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        fs::read_dir(self.base.join(path))
            .map(|entries| {
                entries
                    .filter_map(|entry| {
                        entry
                            .ok()
                            .and_then(|entry| entry.file_name().into_string().ok())
                            .map(SharedString::from)
                    })
                    .collect()
            })
            .map_err(|err| err.into())
    }
}

struct SvgExample;

impl Render for SvgExample {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_row()
            .size_full()
            .justify_center()
            .items_center()
            .gap_8()
            .bg(rgb(0xffffff))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_2()
                    .child(color_svg().path("svg/dragon.svg").size_32())
                    .child("Source colors"),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_2()
                    .child(
                        color_svg()
                            .path("svg/dragon.svg")
                            .size_32()
                            .fill_color(rgb(0x8b5cf6))
                            .with_transformation(Transformation::rotate(radians(-0.12))),
                    )
                    .child("Fill + transform"),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_2()
                    .child(
                        color_svg()
                            .path("svg/color-demo.svg")
                            .size_32()
                            .current_color(rgb(0x22c55e))
                            .text_color(rgb(0xf8fafc)),
                    )
                    .child("currentColor + text_color"),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_2()
                    .child(
                        svg()
                            .path("svg/dragon.svg")
                            .size_32()
                            .text_color(rgb(0xef4444)),
                    )
                    .child("Existing monochrome svg"),
            )
    }
}

fn run_example() {
    application()
        .with_assets(Assets {
            base: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples"),
        })
        .run(|cx: &mut App| {
            let bounds = Bounds::centered(None, size(px(760.0), px(300.0)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |_, cx| cx.new(|_| SvgExample),
            )
            .unwrap();
            cx.activate(true);
        });
}

#[cfg(not(target_family = "wasm"))]
fn main() {
    run_example();
}

#[cfg(target_family = "wasm")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    gpui_platform::web_init();
    run_example();
}
