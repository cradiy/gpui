//! Drag local paths between GPUI windows or into another application.
//! Run with `cargo run -p gpui --example file_drag -- /absolute/path ...`.
use gpui::{prelude::*, *};
use std::{path::PathBuf, sync::Arc};

#[derive(Clone)]
struct Files(Arc<[PathBuf]>);
struct Preview(usize);
impl Render for Preview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .p_3()
            .rounded_md()
            .bg(rgb(0x356bb3))
            .text_color(white())
            .child(format!("{} files", self.0))
    }
}
struct FileDragDemo {
    files: Files,
    status: String,
}
impl Render for FileDragDemo {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let on_end = cx.listener(|this, result: &DragEnd, _, cx| {
            this.status = format!("{result:?}");
            eprintln!("file drag: {result:?}");
            cx.notify();
        });
        div()
            .id("file-drag-demo")
            .size_full()
            .p_6()
            .flex()
            .flex_col()
            .gap_4()
            .bg(rgb(0x20252e))
            .text_color(white())
            .child("Drag files to another window or application")
            .child("Default: Move · Ctrl: Copy · Escape: Cancel")
            .child(
                div()
                    .id("files")
                    .p_4()
                    .bg(rgb(0x356bb3))
                    .rounded_md()
                    .child(format!("Drag {} files", self.files.0.len()))
                    .on_drag(self.files.clone(), |files, _, _, cx| {
                        cx.new(|_| Preview(files.0.len()))
                    })
                    .on_drag_end::<Files>(move |result, _, window, cx| on_end(result, window, cx)),
            )
            .child(
                div()
                    .id("target")
                    .h_24()
                    .p_4()
                    .border_1()
                    .border_color(rgb(0x7897ab))
                    .child("Drop files here · GPUI or another application")
                    .on_drop(cx.listener(|this, files: &Files, _, cx| {
                        this.status = format!("Typed payload: {} paths", files.0.len());
                        cx.notify();
                    }))
                    .on_drop(cx.listener(|this, files: &ExternalPaths, _, cx| {
                        this.status = format!("External files: {} paths", files.paths().len());
                        eprintln!("external files: {:?}", files.paths());
                        cx.notify();
                    })),
            )
            .child(self.status.clone())
            .on_drag_move::<Files>(
                cx.listener(|this, event: &DragMoveEvent<Files>, window, cx| {
                    let point = event.event.position;
                    let size = window.viewport_size();
                    if point.x > px(10.)
                        && point.y > px(10.)
                        && point.x < size.width - px(10.)
                        && point.y < size.height - px(10.)
                    {
                        return;
                    }
                    let files = event.drag(cx).0.clone();
                    if let Err(error) = window.promote_active_file_drag_to_system(
                        files,
                        SystemFileDragOptions::default(),
                        cx,
                    ) {
                        this.status = format!("{error:#}");
                        cx.notify();
                    }
                }),
            )
    }
}
fn main() {
    let paths: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    if paths.is_empty() || paths.iter().any(|path| !path.is_absolute()) {
        eprintln!("Usage: file_drag /absolute/path [/absolute/path ...]");
        return;
    }
    let files = Files(paths.into());
    gpui_platform::application().run(move |cx| {
        for (x, title) in [(80., "File drag A"), (600., "File drag B")] {
            let files = files.clone();
            cx.open_window(
                WindowOptions {
                    titlebar: Some(TitlebarOptions {
                        title: Some(title.into()),
                        ..Default::default()
                    }),
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(x), px(100.)),
                        size(px(480.), px(320.)),
                    ))),
                    ..Default::default()
                },
                |_, cx| {
                    cx.new(|_| FileDragDemo {
                        files,
                        status: "Ready".into(),
                    })
                },
            )
            .unwrap();
        }
        cx.activate(true);
    });
}
