use anyhow::Result;
use gpui::prelude::*;
use gpui::{
    App, AssetSource, Bounds, Context, Entity, Hsla, Render, SharedString, Window, WindowBounds,
    WindowOptions, div, point, px, rgb, size, svg,
};
use gpui_effects::{MotionItem, MotionLayer, MotionOptions, MotionPath, MotionPolicy};
use gpui_platform::application;
use std::{borrow::Cow, fs, path::PathBuf, time::Duration};

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

struct MotionLayerExample {
    motion: Entity<MotionLayer>,
}

impl MotionLayerExample {
    fn new(cx: &mut Context<Self>) -> Self {
        let motion = cx.new(|_| MotionLayer::new());
        let mut example = Self { motion };
        example.launch(cx);
        example
    }

    fn launch(&mut self, cx: &mut Context<Self>) {
        let starts = [
            point(px(80.0), px(130.0)),
            point(px(210.0), px(215.0)),
            point(px(100.0), px(490.0)),
            point(px(380.0), px(555.0)),
            point(px(680.0), px(470.0)),
            point(px(750.0), px(155.0)),
        ];
        let colors = [
            rgb(0xff375f),
            rgb(0xff9f0a),
            rgb(0x30d158),
            rgb(0x64d2ff),
            rgb(0x7c3aed),
            rgb(0xff2d92),
        ];
        let target = point(px(420.0), px(335.0));
        let items = starts.into_iter().enumerate().map(|(index, start)| {
            let color: Hsla = colors[index].into();
            let bend = px((index as f32 - 2.5) * 34.0);
            MotionItem::new(start, target, move |frame| {
                let size = px(34.0);
                div()
                    .absolute()
                    .left(frame.start.x - size * 0.5)
                    .top(frame.start.y - size * 0.5)
                    .size(size)
                    .opacity(frame.fade_out())
                    .child(
                        svg()
                            .path("gradient-mark.svg")
                            .size_full()
                            .text_color(color)
                            .with_transformation(frame.translation()),
                    )
            })
            .delay(Duration::from_millis(index as u64 * 28))
            .path(MotionPath::arc(bend))
        });

        self.motion.update(cx, |motion, cx| {
            motion.start(
                items,
                MotionOptions::new(Duration::from_millis(1_600)).policy(MotionPolicy::Replace),
                cx,
            );
        });
    }
}

impl Render for MotionLayerExample {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let starts = [
            point(px(80.0), px(130.0)),
            point(px(210.0), px(215.0)),
            point(px(100.0), px(490.0)),
            point(px(380.0), px(555.0)),
            point(px(680.0), px(470.0)),
            point(px(750.0), px(155.0)),
        ];

        div()
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(rgb(0x101218))
            .text_color(rgb(0xf5f7ff))
            .children(starts.into_iter().map(|start| {
                div()
                    .absolute()
                    .left(start.x - px(5.0))
                    .top(start.y - px(5.0))
                    .size_2p5()
                    .rounded_full()
                    .border_1()
                    .border_color(rgb(0x5b6475))
            }))
            .child(
                div()
                    .absolute()
                    .left(px(400.0))
                    .top(px(315.0))
                    .size_10()
                    .rounded_full()
                    .border_2()
                    .border_color(rgb(0x00d4aa))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(div().size_2().rounded_full().bg(rgb(0x00d4aa))),
            )
            .child(
                div()
                    .absolute()
                    .top_6()
                    .left_0()
                    .right_0()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_2()
                    .child("Independent MotionLayer · shared arrival time")
                    .child(
                        div()
                            .id("launch-motion")
                            .px_4()
                            .py_2()
                            .rounded_lg()
                            .bg(rgb(0x273142))
                            .hover(|style| style.bg(rgb(0x35435a)))
                            .cursor_pointer()
                            .child("Launch again")
                            .on_click(cx.listener(|this, _, _, cx| this.launch(cx))),
                    ),
            )
            .child(self.motion.clone())
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
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(900.0), px(680.0)),
                        cx,
                    ))),
                    ..Default::default()
                },
                |_, cx| cx.new(MotionLayerExample::new),
            )
            .expect("failed to open motion layer example");
        });
}
