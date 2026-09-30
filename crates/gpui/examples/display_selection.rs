//! Select a display by its index in the startup list.
//! Run with `cargo run -p gpui --example display_selection -- --display 1 --fullscreen`.
//! On Wayland, normal window placement is controlled by the compositor;
//! the selected display is requested when entering fullscreen.

use gpui::{
    App, Bounds, Context, IntoElement, MouseButton, Render, Styled, Window, WindowBounds,
    WindowOptions, div, prelude::*, px, rgb, size,
};
use gpui_platform::application;

struct DisplaySelection {
    selected: String,
}

impl Render for DisplaySelection {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let current = window.display(cx).map(|display| display.id());
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_4()
            .bg(rgb(0x202630))
            .text_color(rgb(0xffffff))
            .child(format!("Requested: {}", self.selected))
            .child(format!("Current: {current:?}"))
            .child(
                div()
                    .id("fullscreen")
                    .px_4()
                    .py_2()
                    .bg(rgb(0x365b91))
                    .rounded_md()
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, |_, window, _| window.toggle_fullscreen())
                    .child("Toggle fullscreen"),
            )
            .child(
                div()
                    .id("quit")
                    .px_4()
                    .py_2()
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.quit())
                    .child("Quit"),
            )
    }
}

fn main() {
    let mut index = 0usize;
    let mut fullscreen = false;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--display" => {
                index = arguments
                    .next()
                    .expect("missing display index")
                    .parse()
                    .expect("invalid display index");
            }
            "--fullscreen" => fullscreen = true,
            _ => panic!("usage: display_selection [--display INDEX] [--fullscreen]"),
        }
    }
    application().run(move |cx: &mut App| {
        let mut displays = cx.displays();
        displays.sort_by_key(|display| u64::from(display.id()));
        for (index, display) in displays.iter().enumerate() {
            println!("{index}: {:?} {:?}", display.id(), display.bounds());
        }
        let display = displays.get(index).expect("display index is not available");
        let bounds = Bounds::centered(Some(display.id()), size(px(640.), px(360.)), cx);
        let selected = format!("{index}: {:?}", display.id());
        cx.open_window(
            WindowOptions {
                display_id: Some(display.id()),
                window_bounds: Some(if fullscreen {
                    WindowBounds::Fullscreen(bounds)
                } else {
                    WindowBounds::Windowed(bounds)
                }),
                ..Default::default()
            },
            |_, cx| cx.new(|_| DisplaySelection { selected }),
        )
        .unwrap();
    });
}
