#![cfg_attr(target_family = "wasm", no_main)]

use gpui::{
    App, Bounds, Context, GlobalShortcut, GlobalShortcutEvent, GlobalShortcutRegistration,
    Keystroke, SharedString, Window, WindowBounds, WindowOptions, div, prelude::*, px, rgb, size,
};
use gpui_platform::application;

struct GlobalShortcutsExample {
    status: SharedString,
    registration: Option<GlobalShortcutRegistration>,
}

impl Render for GlobalShortcutsExample {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_3()
            .size_full()
            .justify_center()
            .items_center()
            .bg(rgb(0x18181b))
            .text_color(rgb(0xf4f4f5))
            .child(div().text_xl().child("Global Shortcuts"))
            .child("Press Ctrl+Shift+Space while this application is in the background.")
            .child(self.status.clone())
    }
}

fn run_example() {
    application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(640.), px(240.)), cx);
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    app_id: Some("dev.gpui.GlobalShortcutsExample".to_string()),
                    ..Default::default()
                },
                |_, cx| {
                    cx.new(|_| GlobalShortcutsExample {
                        status: "Registering…".into(),
                        registration: None,
                    })
                },
            )
            .unwrap();

        cx.observe_global_shortcuts({
            move |event, cx| {
                if let GlobalShortcutEvent::Activated { shortcut_id, .. } = event
                    && shortcut_id.as_ref() == "show-window"
                {
                    window
                        .update(cx, |view, window, cx| {
                            window.activate_window();
                            view.status = "The global shortcut was activated.".into();
                            cx.notify();
                        })
                        .ok();
                }
            }
        })
        .detach();

        let registration = cx.register_global_shortcuts([GlobalShortcut::new(
            "show-window",
            "Show the example window",
            Keystroke::parse("ctrl-shift-space").unwrap(),
        )]);
        cx.spawn(async move |cx| {
            let result = registration.await;
            window
                .update(cx, |view, _, cx| {
                    match result {
                        Ok(registration) => {
                            let trigger = registration.shortcuts()[0].trigger_description();
                            view.status = format!("Registered as {trigger}").into();
                            view.registration = Some(registration);
                        }
                        Err(error) => {
                            view.status = format!("Registration failed: {error:#}").into();
                        }
                    }
                    cx.notify();
                })
                .ok();
        })
        .detach();

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
