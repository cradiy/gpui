//! Copy or cut local file paths, and inspect files pasted from other applications.
//! Run with `cargo run -p gpui --example file_clipboard -- /absolute/path ...`.
use gpui::{prelude::*, *};
use std::path::PathBuf;

struct FileClipboardDemo {
    paths: Vec<PathBuf>,
    status: String,
}

impl Render for FileClipboardDemo {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .p_6()
            .flex()
            .flex_col()
            .gap_4()
            .bg(rgb(0x20252e))
            .text_color(white())
            .child("File clipboard")
            .child(format!(
                "{} source files · Paste into a file manager to copy or move them",
                self.paths.len()
            ))
            .child(
                div().flex().gap_3().children(
                    [
                        ("copy", "Copy files", ClipboardFileOperation::Copy),
                        ("cut", "Cut files", ClipboardFileOperation::Cut),
                    ]
                    .into_iter()
                    .map(|(id, label, operation)| {
                        div()
                            .id(id)
                            .px_4()
                            .py_2()
                            .rounded_md()
                            .bg(rgb(0x356bb3))
                            .cursor_pointer()
                            .child(label)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                match ClipboardItem::new_files(this.paths.clone(), operation) {
                                    Ok(item) => {
                                        cx.write_to_clipboard(item);
                                        this.status =
                                            format!("{operation:?}: {} files", this.paths.len());
                                    }
                                    Err(error) => this.status = format!("{error:#}"),
                                }
                                cx.notify();
                            }))
                    }),
                ),
            )
            .child(
                div()
                    .id("read")
                    .px_4()
                    .py_2()
                    .rounded_md()
                    .bg(rgb(0x34465a))
                    .cursor_pointer()
                    .child("Read clipboard")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.status = match cx.read_from_clipboard() {
                            Some(item) => match item.files() {
                                Some(files) => format!(
                                    "{:?}: {} files\n{}",
                                    files.operation(),
                                    files.paths().len(),
                                    files
                                        .paths()
                                        .iter()
                                        .map(|path| format!("{path:?}"))
                                        .collect::<Vec<_>>()
                                        .join("\n")
                                ),
                                None => format!("No files. Text: {:?}", item.text()),
                            },
                            None => "Clipboard is empty or unavailable".into(),
                        };
                        eprintln!("{}", this.status);
                        cx.notify();
                    })),
            )
            .child("Reading lists the paths and intent; it does not change any files.")
            .child(
                div()
                    .id("status")
                    .flex_1()
                    .overflow_y_scroll()
                    .child(self.status.clone()),
            )
    }
}

fn main() {
    let paths: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    if !paths.is_empty() {
        if let Err(error) = ClipboardItem::new_files(paths.clone(), ClipboardFileOperation::Copy) {
            eprintln!("{error:#}");
            return;
        }
    }
    gpui_platform::application().run(move |cx| {
        cx.open_window(WindowOptions::default(), |_, cx| {
            cx.new(|_| FileClipboardDemo {
                paths,
                status: "Copy or cut files in a file manager, then read the clipboard here.".into(),
            })
        })
        .unwrap();
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        cx.activate(true);
    });
}
